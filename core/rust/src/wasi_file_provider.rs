//! Capability-preserving bridge from Hara [`FileProvider`] values to one
//! preopened WASI Preview 2 directory.
//!
//! WASI's reference host accepts capability directories, whereas Hara owns a
//! provider-neutral filesystem boundary.  This adapter makes the boundary
//! explicit: it snapshots one provider into a private staging directory,
//! preopens only that directory for a Component, and projects deterministic
//! guest changes back after each call.  It never grants the Component an
//! ambient host directory.
//!
//! The source `FileProvider` has no timestamp-restore operation, so metadata
//! timestamps are provider-owned and canonicalize on write.  Content and
//! directory topology are retained; unsupported symlink and special nodes are
//! rejected before instantiation rather than silently flattened.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use crate::file::{
    logical_resolve, DeleteOptions, FileError, FileProvider, FileType, MkdirOptions, WriteMode,
    WriteOptions,
};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Directory,
    File(Vec<u8>),
}

type Tree = BTreeMap<String, Node>;

/// A private capability directory backed by one Hara [`FileProvider`].
///
/// `sync` is idempotent. It first verifies that the provider did not change
/// since the component's prior synchronization, then writes exactly the
/// Component's staged delta. A conflicting host change is rejected without a
/// partial projection.
pub(crate) struct WasiFileProviderProjection {
    provider: Rc<dyn FileProvider>,
    root: PathBuf,
    baseline: Tree,
}

impl WasiFileProviderProjection {
    pub(crate) fn stage(provider: Rc<dyn FileProvider>) -> Result<Self, String> {
        let baseline = provider_tree(provider.as_ref())?;
        let root = create_staging_root()?;
        if let Err(error) = write_staging_tree(&root, &baseline) {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        Ok(Self {
            provider,
            root,
            baseline,
        })
    }

    pub(crate) fn preopened_dir(&self) -> Result<Dir, String> {
        Dir::open_ambient_dir(&self.root, ambient_authority()).map_err(|error| {
            format!(
                "extension/file-provider-unavailable: cannot open staged WASI directory: {error}"
            )
        })
    }

    pub(crate) fn sync(&mut self) -> Result<(), String> {
        let staged = staging_tree(&self.root)?;
        let current = provider_tree(self.provider.as_ref())?;
        let changed_paths = changed_paths(&self.baseline, &staged);
        for path in &changed_paths {
            if current.get(path) != self.baseline.get(path) {
                return Err(format!(
                    "extension/filesystem-conflict: FileProvider changed {path} while the Component held its WASI projection"
                ));
            }
        }

        let mut removals = self
            .baseline
            .iter()
            .filter_map(|(path, before)| match staged.get(path) {
                None => Some(path.clone()),
                Some(after) if std::mem::discriminant(before) != std::mem::discriminant(after) => {
                    Some(path.clone())
                }
                Some(_) => None,
            })
            .collect::<Vec<_>>();
        removals.sort_by_key(|path| std::cmp::Reverse(path_depth(path)));
        for path in removals {
            self.provider
                .delete_path(&path, DeleteOptions { missing_ok: true })
                .map_err(|error| provider_error("delete", &path, error))?;
        }

        let mut directories = staged
            .iter()
            .filter_map(|(path, node)| {
                matches!(node, Node::Directory)
                    .then(|| self.baseline.get(path) != Some(node))
                    .and_then(|changed| changed.then(|| path.clone()))
            })
            .collect::<Vec<_>>();
        directories.sort_by_key(|path| path_depth(path));
        for path in directories {
            if path == "/" {
                continue;
            }
            self.provider
                .mkdir_path(
                    &path,
                    MkdirOptions {
                        parents: true,
                        exists_ok: true,
                    },
                )
                .map_err(|error| provider_error("mkdir", &path, error))?;
        }

        for (path, node) in &staged {
            let Node::File(bytes) = node else {
                continue;
            };
            if self.baseline.get(path) == Some(node) {
                continue;
            }
            self.provider
                .write_bytes(
                    path,
                    bytes.clone(),
                    WriteOptions {
                        mode: WriteMode::Replace,
                        parents: true,
                    },
                )
                .map_err(|error| provider_error("write", path, error))?;
        }

        self.baseline = staged;
        Ok(())
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for WasiFileProviderProjection {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn create_staging_root() -> Result<PathBuf, String> {
    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "hara-wasi-file-provider-{}-{sequence:016x}",
            std::process::id()
        ));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "extension/file-provider-unavailable: cannot create WASI staging directory: {error}"
                ))
            }
        }
    }
    Err("extension/file-provider-unavailable: cannot allocate WASI staging directory".into())
}

fn provider_tree(provider: &dyn FileProvider) -> Result<Tree, String> {
    let root = provider
        .stat_entry("/")
        .map_err(|error| provider_error("stat", "/", error))?;
    if root.kind != FileType::Directory {
        return Err(
            "extension/file-provider-invalid: FileProvider root must be a directory".into(),
        );
    }
    let mut tree = Tree::from([("/".into(), Node::Directory)]);
    collect_provider_tree(provider, "/", &mut tree)?;
    Ok(tree)
}

fn collect_provider_tree(
    provider: &dyn FileProvider,
    path: &str,
    tree: &mut Tree,
) -> Result<(), String> {
    let entries = provider
        .entries_values(path)
        .map_err(|error| provider_error("entries", path, error))?;
    for entry in entries {
        match entry.kind {
            FileType::Directory => {
                if tree.insert(entry.path.clone(), Node::Directory).is_some() {
                    return Err(format!(
                        "extension/file-provider-invalid: duplicate filesystem path {}",
                        entry.path
                    ));
                }
                collect_provider_tree(provider, &entry.path, tree)?;
            }
            FileType::File => {
                let bytes = provider
                    .read_bytes(&entry.path)
                    .map_err(|error| provider_error("read", &entry.path, error))?;
                if tree.insert(entry.path.clone(), Node::File(bytes)).is_some() {
                    return Err(format!(
                        "extension/file-provider-invalid: duplicate filesystem path {}",
                        entry.path
                    ));
                }
            }
            FileType::Symlink | FileType::Other => {
                return Err(format!(
                    "extension/file-provider-unsupported: cannot project {} at {} into WASI",
                    entry.kind.keyword(),
                    entry.path
                ))
            }
        }
    }
    Ok(())
}

fn write_staging_tree(root: &Path, tree: &Tree) -> Result<(), String> {
    for (path, node) in tree {
        if path == "/" {
            continue;
        }
        let target = staged_path(root, path)?;
        match node {
            Node::Directory => fs::create_dir_all(&target),
            Node::File(bytes) => {
                let parent = target.parent().ok_or_else(|| {
                    format!("extension/file-provider-invalid: staged file has no parent {path}")
                })?;
                fs::create_dir_all(parent).and_then(|()| fs::write(&target, bytes))
            }
        }
        .map_err(|error| {
            format!("extension/file-provider-unavailable: cannot stage {path}: {error}")
        })?;
    }
    Ok(())
}

fn staging_tree(root: &Path) -> Result<Tree, String> {
    let mut tree = Tree::from([("/".into(), Node::Directory)]);
    collect_staging_tree(root, "/", &mut tree)?;
    Ok(tree)
}

fn collect_staging_tree(root: &Path, path: &str, tree: &mut Tree) -> Result<(), String> {
    let directory = staged_path(root, path)?;
    let mut entries = fs::read_dir(&directory)
        .map_err(|error| {
            format!("extension/file-provider-unavailable: cannot read staged {path}: {error}")
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!("extension/file-provider-unavailable: cannot read staged {path}: {error}")
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|_| {
            format!("extension/file-provider-invalid: staged WASI path is not UTF-8 under {path}")
        })?;
        let child =
            logical_resolve(path, &name).map_err(|error| provider_error("resolve", path, error))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!("extension/file-provider-unavailable: cannot stat staged {child}: {error}")
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
            return Err(format!(
                "extension/file-provider-unsupported: Component created unsupported filesystem node {child}"
            ));
        }
        if metadata.is_dir() {
            tree.insert(child.clone(), Node::Directory);
            collect_staging_tree(root, &child, tree)?;
        } else {
            let bytes = fs::read(entry.path()).map_err(|error| {
                format!("extension/file-provider-unavailable: cannot read staged {child}: {error}")
            })?;
            tree.insert(child, Node::File(bytes));
        }
    }
    Ok(())
}

fn staged_path(root: &Path, logical: &str) -> Result<PathBuf, String> {
    let relative = logical.strip_prefix('/').ok_or_else(|| {
        format!("extension/file-provider-invalid: expected an absolute Hara path, got {logical}")
    })?;
    Ok(root.join(relative))
}

fn changed_paths(before: &Tree, after: &Tree) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

fn path_depth(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}

fn provider_error(operation: &str, path: &str, error: FileError) -> String {
    format!("extension/file-provider-{operation}: {path} ({error:?})")
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::file::{FileProvider, MemoryFileProvider, WriteMode, WriteOptions};

    use super::WasiFileProviderProjection;

    #[test]
    fn stages_and_synchronizes_a_memory_file_provider_without_ambient_access() {
        let provider = MemoryFileProvider::new("/");
        provider.insert("/input.md", b"# heading".to_vec()).unwrap();
        let mut projection = WasiFileProviderProjection::stage(Rc::new(provider.clone())).unwrap();
        assert_eq!(
            std::fs::read(projection.path().join("input.md")).unwrap(),
            b"# heading"
        );
        std::fs::write(projection.path().join("output.html"), b"<h1>heading</h1>").unwrap();

        projection.sync().unwrap();
        projection.sync().unwrap();

        assert_eq!(
            provider.read_bytes("/output.html").unwrap(),
            b"<h1>heading</h1>"
        );
    }

    #[test]
    fn removes_deleted_component_files_from_the_provider_projection() {
        let provider = MemoryFileProvider::new("/");
        provider.insert("/obsolete.md", b"old".to_vec()).unwrap();
        let mut projection = WasiFileProviderProjection::stage(Rc::new(provider.clone())).unwrap();
        std::fs::remove_file(projection.path().join("obsolete.md")).unwrap();

        projection.sync().unwrap();

        assert!(!provider.exists_value("/obsolete.md").unwrap());
    }

    #[test]
    fn rejects_provider_changes_that_conflict_with_a_guest_projection() {
        let provider = MemoryFileProvider::new("/");
        provider.insert("/document.md", b"before".to_vec()).unwrap();
        let mut projection = WasiFileProviderProjection::stage(Rc::new(provider.clone())).unwrap();
        std::fs::write(projection.path().join("document.md"), b"guest").unwrap();
        provider
            .write_bytes(
                "/document.md",
                b"host".to_vec(),
                WriteOptions {
                    mode: WriteMode::Replace,
                    parents: true,
                },
            )
            .unwrap();

        assert!(projection
            .sync()
            .unwrap_err()
            .contains("filesystem-conflict"));
        assert_eq!(provider.read_bytes("/document.md").unwrap(), b"host");
    }
}
