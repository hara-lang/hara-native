use super::{declared_namespace, files_in, Project};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

#[path = "resources/installed.rs"]
mod installed;

/// A source-only namespace catalog used by native runtimes.
///
/// The catalog resolves conventional namespace paths at the `require`
/// boundary. It discovers the complete legacy path map only when a requested
/// namespace cannot be derived from its name, so starting a project never
/// walks unrelated source families such as `lang.*`.
#[derive(Debug, Clone, Default)]
pub struct SourceCatalog {
    entries: Arc<Mutex<BTreeMap<String, PathBuf>>>,
    roots: Vec<PathBuf>,
    excluded_roots: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(FileStamp {
        length: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

impl SourceCatalog {
    /// Completes the legacy map for callers that need package-wide metadata.
    /// Runtime namespace loading should call `path` instead.
    pub(crate) fn entries(&self) -> BTreeMap<String, PathBuf> {
        self.discover_legacy_paths();
        self.entries
            .lock()
            .expect("source catalog cache poisoned")
            .clone()
    }

    /// Resolves one namespace on demand. Conventional `foo.bar` namespaces
    /// use `foo/bar.hal` directly; a one-time legacy scan is reserved for
    /// paths such as `impl_base.hal` declaring `impl-base`.
    pub fn path(&self, namespace: &str) -> Option<PathBuf> {
        if let Some(path) = self
            .entries
            .lock()
            .expect("source catalog cache poisoned")
            .get(namespace)
            .cloned()
        {
            return Some(path);
        }
        if let Some(path) = self.conventional_path(namespace) {
            self.entries
                .lock()
                .expect("source catalog cache poisoned")
                .insert(namespace.to_owned(), path.clone());
            return Some(path);
        }
        self.discover_legacy_paths();
        self.entries
            .lock()
            .expect("source catalog cache poisoned")
            .get(namespace)
            .cloned()
    }

    pub fn namespaces(&self) -> Vec<String> {
        self.entries().into_keys().collect()
    }

    /// Returns a stable fingerprint of the indexed source set. The source
    /// cache keys individual programs by source bytes as well; this broader
    /// index fingerprint invalidates programs whose compilation depends on a
    /// changed sibling namespace configuration without rereading every source
    /// body during ordinary namespace loading.
    pub fn fingerprint(&self) -> Result<[u8; 32], String> {
        let mut digest = Sha256::new();
        digest.update(b"hara-source-index-v1\0");
        for (namespace, path) in self.entries() {
            let stamp = file_stamp(&path)
                .ok_or_else(|| format!("cannot stat source file {}", path.display()))?;
            digest.update(namespace.as_bytes());
            digest.update([0]);
            digest.update(path.to_string_lossy().as_bytes());
            digest.update([0]);
            digest.update(stamp.length.to_le_bytes());
            digest.update(stamp.modified_seconds.to_le_bytes());
            digest.update(stamp.modified_nanos.to_le_bytes());
        }
        Ok(digest.finalize().into())
    }

    /// Returns a content address for source namespaces selected by a family
    /// prefix.  Artifact owners use this instead of the whole-project index
    /// when unrelated application source must not invalidate their cache.
    pub fn content_fingerprint_prefixes(
        &self,
        prefixes: &[&str],
    ) -> Result<[u8; 32], String> {
        let mut selected = BTreeSet::new();
        for (namespace, path) in self.entries() {
            if prefixes.iter().any(|prefix| {
                namespace == *prefix
                    || namespace
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            }) {
                selected.insert((namespace, path));
            }
        }

        let mut digest = Sha256::new();
        digest.update(b"hara-source-content-family-v1\0");
        for (namespace, path) in selected {
            let source = fs::read(&path)
                .map_err(|error| format!("cannot read source file {}: {error}", path.display()))?;
            digest.update(namespace.as_bytes());
            digest.update([0]);
            digest.update(source.len().to_le_bytes());
            digest.update(source);
        }
        Ok(digest.finalize().into())
    }

    /// Returns a content address for one namespace and its declarative source
    /// dependency closure. Unlike family fingerprints, this resolves only the
    /// namespaces that an artifact can load, so a JavaScript Book is not
    /// invalidated by unrelated Python, Lua, or application source.
    pub fn content_fingerprint_dependencies(
        &self,
        roots: &[&str],
    ) -> Result<[u8; 32], String> {
        let requested = roots
            .iter()
            .map(|namespace| (*namespace).to_owned())
            .collect::<BTreeSet<_>>();
        let mut pending = requested.clone();
        let mut selected = BTreeMap::new();

        while let Some(namespace) = pending.iter().next().cloned() {
            pending.remove(&namespace);
            if selected.contains_key(&namespace) {
                continue;
            }
            let Some(path) = self.path(&namespace) else {
                if requested.contains(&namespace) {
                    return Err(format!("cannot resolve source namespace {namespace}"));
                }
                continue;
            };
            let source = fs::read(&path)
                .map_err(|error| format!("cannot read source file {}: {error}", path.display()))?;
            for dependency in source_namespace_dependencies(&source, &path)? {
                if !selected.contains_key(&dependency) {
                    pending.insert(dependency);
                }
            }
            selected.insert(namespace, source);
        }

        let mut digest = Sha256::new();
        digest.update(b"hara-source-content-closure-v1\0");
        for (namespace, source) in selected {
            digest.update(namespace.as_bytes());
            digest.update([0]);
            digest.update(source.len().to_le_bytes());
            digest.update(source);
        }
        Ok(digest.finalize().into())
    }

    pub(crate) fn cached_namespaces(&self) -> Vec<String> {
        self.entries
            .lock()
            .expect("source catalog cache poisoned")
            .keys()
            .cloned()
            .collect()
    }

    fn add_project(&mut self, project: &Project) -> Result<(), String> {
        let project_root = project
            .root
            .canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", project.root.display()))?;
        for excluded_root in &project.source_excludes {
            let excluded_root = project.root.join(excluded_root);
            if !excluded_root.exists() {
                continue;
            }
            let excluded_root = excluded_root.canonicalize().map_err(|error| {
                format!(
                    "cannot resolve excluded source root {}: {error}",
                    excluded_root.display()
                )
            })?;
            if !excluded_root.starts_with(&project_root) {
                return Err(format!(
                    "excluded source root escapes project root: {}",
                    excluded_root.display()
                ));
            }
            self.excluded_roots.push(excluded_root);
        }
        for source_root in &project.source_paths {
            let source_root = project.root.join(source_root);
            if !source_root.exists() {
                continue;
            }
            let source_root = source_root.canonicalize().map_err(|error| {
                format!(
                    "cannot resolve source root {}: {error}",
                    source_root.display()
                )
            })?;
            if !source_root.starts_with(&project_root) {
                return Err(format!(
                    "source root escapes project root: {}",
                    source_root.display()
                ));
            }
            self.roots.push(source_root);
        }
        Ok(())
    }

    fn excluded(&self, path: &Path) -> bool {
        self.excluded_roots
            .iter()
            .any(|excluded_root| path.starts_with(excluded_root))
    }

    fn conventional_path(&self, namespace: &str) -> Option<PathBuf> {
        let segments = namespace.split('.').collect::<Vec<_>>();
        if segments.is_empty()
            || segments.iter().any(|segment| {
                segment.is_empty()
                    || *segment == ".."
                    || segment.contains('/')
                    || segment.contains('\\')
            })
        {
            return None;
        }
        for root in self.roots.iter().rev() {
            for underscores in [false, true] {
                let mut candidate = root.to_path_buf();
                for segment in &segments[..segments.len().saturating_sub(1)] {
                    candidate.push(if underscores {
                        segment.replace('-', "_")
                    } else {
                        (*segment).to_owned()
                    });
                }
                let leaf = segments.last().expect("non-empty segments");
                candidate.push(format!(
                    "{}.hal",
                    if underscores {
                        leaf.replace('-', "_")
                    } else {
                        (*leaf).to_owned()
                    }
                ));
                let Ok(path) = candidate.canonicalize() else {
                    continue;
                };
                if path.starts_with(root) && path.is_file() && !self.excluded(&path) {
                    return Some(path);
                }
            }
        }
        None
    }

    fn discover_legacy_paths(&self) {
        let mut discovered = BTreeMap::new();
        for root in &self.roots {
            let Ok(paths) = files_in(root, &[PathBuf::from(".")]) else {
                continue;
            };
            for path in paths {
                let Ok(path) = path.canonicalize() else {
                    continue;
                };
                if !path.starts_with(root) || self.excluded(&path) {
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(Some(namespace)) = declared_namespace_header(&source) else {
                    continue;
                };
                // Project layers are ordered from dependencies to the active
                // project, so the later mapping is the intentional overlay.
                discovered.insert(namespace, path);
            }
        }
        self.entries
            .lock()
            .expect("source catalog cache poisoned")
            .extend(discovered);
    }
}

fn source_namespace_dependencies(source: &[u8], path: &Path) -> Result<Vec<String>, String> {
    let source = std::str::from_utf8(source)
        .map_err(|error| format!("cannot decode source file {}: {error}", path.display()))?;
    let forms = crate::kernel::read_forms(source)
        .map_err(|error| format!("cannot parse source file {}: {error}", path.display()))?;
    for form in forms {
        let crate::kernel::Form::List(values) = resource_without_metadata(&form.form) else {
            continue;
        };
        if !matches!(values.first(), Some(crate::kernel::Form::Symbol(head)) if head == "ns" || head == "ns+") {
            continue;
        }
        let config = crate::kernel::GeneratedNamespaceConfig::configure_with(&values[2..], |_| true)
            .map_err(|error| format!("cannot read namespace dependencies from {}: {error}", path.display()))?;
        let mut dependencies = config.required_namespaces().to_vec();
        dependencies.extend(config.used_namespaces().iter().cloned());
        dependencies.sort();
        dependencies.dedup();
        return Ok(dependencies);
    }
    Ok(Vec::new())
}

fn resource_without_metadata(form: &crate::kernel::Form) -> &crate::kernel::Form {
    match form {
        crate::kernel::Form::Metadata(_, value) => resource_without_metadata(value),
        value => value,
    }
}

fn declared_namespace_header(source: &str) -> Result<Option<String>, String> {
    let mut depth = 0;
    let mut form_start = None;
    let mut in_comment = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut skip_character = false;
    for (index, character) in source.char_indices() {
        if skip_character {
            skip_character = false;
            continue;
        }
        if in_comment {
            if character == '\n' {
                in_comment = false;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            ';' => in_comment = true,
            '"' => in_string = true,
            '\\' => skip_character = true,
            '(' | '[' | '{' => {
                if depth == 0 && character == '(' {
                    form_start = Some(index);
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = form_start.take() {
                        let end = index + character.len_utf8();
                        if let Some(namespace) = declared_namespace(&source[start..end])? {
                            return Ok(Some(namespace));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Builds a path-backed source catalog for one project and its installed Hara
/// dependencies.
pub fn source_catalog(project: &Project) -> Result<SourceCatalog, String> {
    source_catalog_at(project, &dist_root())
}

/// Builds a source catalog for one project using packages installed beneath an
/// explicit distribution root. Embedders use this to keep an isolated package
/// store from falling back to the user's global installation.
pub fn source_catalog_at(
    project: &Project,
    distribution_root: &Path,
) -> Result<SourceCatalog, String> {
    source_catalogs_at(&[project], distribution_root)
}

/// Builds a path-backed source catalog for several ordered project layers.
/// Each project contributes its verified installed dependencies first,
/// followed by its own source paths; later project layers take precedence.
pub fn source_catalogs(projects: &[&Project]) -> Result<SourceCatalog, String> {
    source_catalogs_at(projects, &dist_root())
}

/// Builds a multi-project source catalog using one explicit installed-package
/// store. This keeps dependency resolution deterministic for isolated hosts,
/// release verification, and native conformance tests.
pub(crate) fn source_catalogs_at(
    projects: &[&Project],
    distribution_root: &Path,
) -> Result<SourceCatalog, String> {
    let mut catalog = SourceCatalog::default();
    for project in projects {
        for dependency in installed::resolve(project, distribution_root)? {
            catalog.add_project(&dependency.project)?;
        }
        catalog.add_project(project)?;
    }
    Ok(catalog)
}

/// Returns namespace resources from installed dependencies followed by the
/// automatically selected native Rust profile of the consuming project.
pub fn source_resources(project: &Project) -> Result<Vec<(String, String)>, String> {
    source_resources_at(project, &dist_root())
}

pub(crate) fn source_resources_at(
    project: &Project,
    distribution_root: &Path,
) -> Result<Vec<(String, String)>, String> {
    let mut resources = Vec::new();
    let mut declarations = BTreeMap::<String, (String, PathBuf)>::new();
    for dependency in installed::resolve(project, distribution_root)? {
        collect_project(
            &dependency.project,
            &format!("{}@{}", dependency.coordinate, dependency.version),
            &mut declarations,
            &mut resources,
        )?;
    }
    collect_project(
        project,
        &format!("{}@{}", project.id, project.version),
        &mut declarations,
        &mut resources,
    )?;
    Ok(resources)
}

fn collect_project(
    project: &Project,
    owner: &str,
    declarations: &mut BTreeMap<String, (String, PathBuf)>,
    resources: &mut Vec<(String, String)>,
) -> Result<(), String> {
    for path in files_in(&project.root, &project.source_paths)? {
        if project
            .source_excludes
            .iter()
            .any(|excluded_root| path.starts_with(project.root.join(excluded_root)))
        {
            continue;
        }
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let namespace = declared_namespace(&source)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .ok_or_else(|| format!("{} does not declare an ns or ns+ namespace", path.display()))?;
        if let Some((previous_owner, previous_path)) =
            declarations.insert(namespace.clone(), (owner.to_owned(), path.clone()))
        {
            return Err(format!(
                "duplicate namespace {namespace}: {previous_owner} ({}) and {owner} ({})",
                previous_path.display(),
                path.display()
            ));
        }
        resources.push((namespace, source));
    }
    Ok(())
}

fn dist_root() -> PathBuf {
    if let Some(root) = std::env::var_os("HARA_DIST_HOME") {
        return PathBuf::from(root);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hara/dist")
}

#[cfg(test)]
mod tests {
    use super::SourceCatalog;
    use std::fs;

    #[test]
    fn dependency_fingerprint_excludes_unrelated_source() {
        let root = std::env::temp_dir().join(format!(
            "hara-source-catalog-dependency-fingerprint-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("fixture")).unwrap();
        fs::create_dir_all(root.join("unrelated")).unwrap();
        fs::write(
            root.join("fixture/entry_point.hal"),
            "(ns fixture.entry-point (:require [fixture.helper-value :as helper]))\n(def value helper/value)\n",
        )
        .unwrap();
        fs::write(
            root.join("fixture/helper_value.hal"),
            "(ns fixture.helper-value)\n(def value 1)\n",
        )
        .unwrap();
        fs::write(root.join("unrelated/value.hal"), "(ns unrelated.value)\n(def value 1)\n")
            .unwrap();
        let catalog = SourceCatalog {
            entries: Default::default(),
            roots: vec![root.canonicalize().unwrap()],
            excluded_roots: Vec::new(),
        };

        let initial = catalog
            .content_fingerprint_dependencies(&["fixture.entry-point"])
            .unwrap();
        fs::write(root.join("unrelated/value.hal"), "(ns unrelated.value)\n(def value 2)\n")
            .unwrap();
        assert_eq!(
            catalog
                .content_fingerprint_dependencies(&["fixture.entry-point"])
                .unwrap(),
            initial
        );
        fs::write(
            root.join("fixture/helper_value.hal"),
            "(ns fixture.helper-value)\n(def value 2)\n",
        )
        .unwrap();
        assert_ne!(
            catalog
                .content_fingerprint_dependencies(&["fixture.entry-point"])
                .unwrap(),
            initial
        );

        fs::remove_dir_all(root).unwrap();
    }
}
