use super::{declared_namespace, files_in, Project};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
            let mut candidate = root.to_path_buf();
            for segment in &segments[..segments.len().saturating_sub(1)] {
                candidate.push(segment);
            }
            candidate.push(format!(
                "{}.hal",
                segments.last().expect("non-empty segments")
            ));
            let path = candidate.canonicalize().ok()?;
            if path.starts_with(root) && path.is_file() {
                return Some(path);
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
                if !path.starts_with(root) {
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
    source_catalogs(&[project])
}

/// Builds a path-backed source catalog for several ordered project layers.
/// Each project contributes its verified installed dependencies first,
/// followed by its own source paths; later project layers take precedence.
pub fn source_catalogs(projects: &[&Project]) -> Result<SourceCatalog, String> {
    let distribution_root = dist_root();
    let mut catalog = SourceCatalog::default();
    for project in projects {
        for dependency in installed::resolve(project, &distribution_root)? {
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
