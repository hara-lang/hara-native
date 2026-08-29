use super::{lookup, map, LockMode, Project};
use crate::kernel::{parse, Form};
#[cfg(not(target_arch = "wasm32"))]
use crate::Runtime;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_WASM_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockedPackage {
    path: String,
    version: String,
    integrity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockedImport {
    package: String,
    module: String,
    abi: String,
    sha256: String,
    size: u64,
    cache: String,
}

pub(super) fn sync(project: &Project, mode: LockMode, lock: &Path) -> Result<PathBuf, String> {
    if matches!(mode, LockMode::Locked | LockMode::Frozen) {
        validate_locked_cache(project, lock)
            .map_err(|error| format!("{}: {error}", mode.flag()))?;
        return Ok(lock.to_path_buf());
    }

    reconcile(project, mode == LockMode::Offline)?;
    let source = resolve_local(project)?;
    fs::write(lock, source).map_err(|error| format!("cannot write {}: {error}", lock.display()))?;
    Ok(lock.to_path_buf())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn install(project: &Project, runtime: &mut Runtime) -> Result<(), String> {
    let lock = project.root.join("project.lock.edn");
    for (logical, import) in locked_imports(project, &lock)? {
        let bytes = verified_cache_bytes(project, &logical, &import)?;
        runtime.install_direct_wasm_import(&logical, &bytes)?;
    }
    Ok(())
}

pub(super) fn validate_locked_cache(project: &Project, lock: &Path) -> Result<(), String> {
    let imports = locked_imports(project, lock)?;
    for (logical, import) in imports {
        verified_cache_bytes(project, &logical, &import)?;
    }
    Ok(())
}

pub(super) fn archive_entries(project: &Project) -> Result<Vec<PathBuf>, String> {
    let lock = project.root.join("project.lock.edn");
    let imports = locked_imports(project, &lock)?;
    let mut paths = imports
        .into_values()
        .map(|import| PathBuf::from(import.cache))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    for path in &paths {
        let logical = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("cached WASM artifact");
        let import = LockedImport {
            package: String::new(),
            module: String::new(),
            abi: String::new(),
            sha256: logical.to_owned(),
            size: fs::metadata(project.root.join(path))
                .map_err(|error| format!("npm/cache-unavailable: {} ({error})", path.display()))?
                .len(),
            cache: path_text(path)?,
        };
        verified_cache_bytes(project, logical, &import)?;
    }
    Ok(paths)
}

fn locked_imports(
    project: &Project,
    lock: &Path,
) -> Result<BTreeMap<String, LockedImport>, String> {
    let source = fs::read_to_string(lock)
        .map_err(|error| format!("npm/lock-unavailable: {} ({error})", lock.display()))?;
    let document = parse(&source).map_err(|error| format!("npm/lock-invalid: {error}"))?;
    let root = map(&document, "npm/lock-invalid: root must be a map")?;
    if !matches!(lookup(root, "lock/format"), Some(Form::String(value)) if value == "0.0.0-alpha") {
        return Err("npm/lock-invalid: unsupported :lock/format".into());
    }
    let runtime = map(
        lookup(root, "runtime").ok_or("npm/lock-invalid: missing :runtime")?,
        "npm/lock-invalid: :runtime must be a map",
    )?;
    let rust = map(
        lookup(runtime, "rust").ok_or("npm/lock-invalid: missing :runtime :rust")?,
        "npm/lock-invalid: :rust must be a map",
    )?;
    let entries = map(
        lookup(rust, "imports").ok_or("npm/lock-invalid: missing :imports")?,
        "npm/lock-invalid: :imports must be a map",
    )?;
    let mut imports = BTreeMap::new();
    for (key, value) in entries {
        let logical = match key {
            Form::Symbol(value) => value.clone(),
            _ => return Err("npm/lock-invalid: import keys must be symbols".into()),
        };
        let fields = map(value, "npm/lock-invalid: import entry must be a map")?;
        let text = |name: &str| -> Result<String, String> {
            match lookup(fields, name) {
                Some(Form::String(value)) => Ok(value.clone()),
                _ => Err(format!(
                    "npm/lock-invalid: {logical} requires string :{name}"
                )),
            }
        };
        let abi = match lookup(fields, "abi") {
            Some(Form::Keyword(value)) => value.clone(),
            _ => return Err(format!("npm/lock-invalid: {logical} requires keyword :abi")),
        };
        let size = match lookup(fields, "size") {
            Some(Form::Number(value)) if *value >= 0 => *value as u64,
            _ => {
                return Err(format!(
                    "npm/lock-invalid: {logical} requires non-negative :size"
                ))
            }
        };
        let sha256 = text("sha256")?
            .strip_prefix("sha256:")
            .ok_or_else(|| format!("npm/lock-invalid: {logical} has invalid :sha256"))?
            .to_owned();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("npm/lock-invalid: {logical} has invalid :sha256"));
        }
        let import = LockedImport {
            package: text("package")?,
            module: text("module")?,
            abi,
            sha256,
            size,
            cache: text("cache")?,
        };
        let declared = project
            .native_imports
            .get(&logical)
            .ok_or_else(|| format!("npm/lock-stale: undeclared import {logical}"))?;
        if import.package != declared.package
            || import.module != path_text(&declared.module)?
            || import.abi != declared.abi
        {
            return Err(format!("npm/lock-stale: declaration changed for {logical}"));
        }
        imports.insert(logical, import);
    }
    if imports.len() != project.native_imports.len() {
        return Err("npm/lock-stale: import set differs from project.edn".into());
    }
    Ok(imports)
}

fn verified_cache_bytes(
    project: &Project,
    logical: &str,
    import: &LockedImport,
) -> Result<Vec<u8>, String> {
    let relative = Path::new(&import.cache);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!("npm/cache-path-denied: {logical}"));
    }
    let expected = PathBuf::from(format!(
        "target/hara/npm/content/sha256/{}.wasm",
        import.sha256
    ));
    if relative != expected {
        return Err(format!("npm/cache-path-mismatch: {logical}"));
    }
    let bytes = fs::read(project.root.join(relative))
        .map_err(|error| format!("npm/cache-unavailable: {logical} ({error})"))?;
    if bytes.len() as u64 != import.size {
        return Err(format!("npm/cache-size-mismatch: {logical}"));
    }
    if bytes.len() as u64 > MAX_WASM_BYTES || bytes.get(..4) != Some(b"\0asm") {
        return Err(format!("npm/cache-invalid-wasm: {logical}"));
    }
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != import.sha256 {
        return Err(format!("npm/cache-digest-mismatch: {logical}"));
    }
    Ok(bytes)
}

fn reconcile(project: &Project, offline: bool) -> Result<(), String> {
    let workspace = workspace(project);
    fs::create_dir_all(&workspace)
        .map_err(|error| format!("cannot create npm WASM workspace: {error}"))?;
    let dependencies = project
        .npm_dependencies
        .iter()
        .map(|(name, dependency)| (name.clone(), Value::String(dependency.version.to_string())))
        .collect::<Map<_, _>>();
    let package = serde_json::json!({
        "name": "hara-direct-wasm-resolution",
        "private": true,
        "version": "0.0.0",
        "dependencies": dependencies
    });
    fs::write(
        workspace.join("package.json"),
        serde_json::to_vec_pretty(&package).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write npm WASM package.json: {error}"))?;

    run_npm(&workspace, &["install", "--package-lock-only"], offline)?;
    verify_declared_integrity(project, &workspace.join("package-lock.json"))?;
    run_npm(&workspace, &["ci"], offline)?;
    Ok(())
}

fn run_npm(workspace: &Path, operation: &[&str], offline: bool) -> Result<(), String> {
    let mut command = Command::new("npm");
    command.current_dir(workspace).args(operation).args([
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--save-exact",
    ]);
    if offline {
        command.arg("--offline");
    }
    let output = command
        .output()
        .map_err(|error| format!("npm/acquisition-unavailable: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("npm/acquisition-failed: {}", detail.trim()));
    }
    Ok(())
}

fn resolve_local(project: &Project) -> Result<String, String> {
    let workspace = workspace(project);
    let lock_path = workspace.join("package-lock.json");
    verify_declared_integrity(project, &lock_path)?;
    let packages = lock_packages(&lock_path)?;
    let mut imports = BTreeMap::new();
    for (logical, declaration) in &project.native_imports {
        let package_root = workspace.join("node_modules").join(&declaration.package);
        let canonical_root = package_root.canonicalize().map_err(|error| {
            format!("npm/module-unavailable: {} ({error})", declaration.package)
        })?;
        let selected = package_root
            .join(&declaration.module)
            .canonicalize()
            .map_err(|error| format!("npm/module-unavailable: {logical} ({error})"))?;
        if !selected.starts_with(&canonical_root) || !selected.is_file() {
            return Err(format!("npm/path-denied: {logical}"));
        }
        let metadata = selected
            .metadata()
            .map_err(|error| format!("npm/module-unavailable: {logical} ({error})"))?;
        if metadata.len() > MAX_WASM_BYTES {
            return Err(format!("npm/module-too-large: {logical}"));
        }
        let bytes = fs::read(&selected)
            .map_err(|error| format!("npm/module-unavailable: {logical} ({error})"))?;
        if bytes.get(..4) != Some(b"\0asm") {
            return Err(format!("npm/module-invalid-media-type: {logical}"));
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let relative_cache = format!("target/hara/npm/content/sha256/{sha256}.wasm");
        let cache = project.root.join(&relative_cache);
        fs::create_dir_all(cache.parent().expect("cache file has a parent"))
            .map_err(|error| format!("cannot create npm content cache: {error}"))?;
        if cache.is_file() {
            let cached = fs::read(&cache)
                .map_err(|error| format!("cannot read npm content cache: {error}"))?;
            if cached != bytes {
                return Err(format!("npm/cache-digest-conflict: {sha256}"));
            }
        } else {
            fs::write(&cache, &bytes)
                .map_err(|error| format!("cannot write npm content cache: {error}"))?;
        }
        imports.insert(
            logical.clone(),
            LockedImport {
                package: declaration.package.clone(),
                module: path_text(&declaration.module)?,
                abi: declaration.abi.clone(),
                sha256,
                size: metadata.len(),
                cache: relative_cache,
            },
        );
    }
    Ok(render_lock(&packages, &imports))
}

fn verify_declared_integrity(project: &Project, lock_path: &Path) -> Result<(), String> {
    let packages = lock_packages(lock_path)?;
    for (name, declaration) in &project.npm_dependencies {
        let path = format!("node_modules/{name}");
        let locked = packages
            .get(&path)
            .ok_or_else(|| format!("npm/lock-missing: {name}"))?;
        if locked.version != declaration.version.to_string() {
            return Err(format!("npm/version-mismatch: {name}"));
        }
        if locked.integrity != declaration.integrity {
            return Err(format!("npm/integrity-mismatch: {name}"));
        }
    }
    Ok(())
}

fn lock_packages(path: &Path) -> Result<BTreeMap<String, LockedPackage>, String> {
    let bytes = fs::read(path).map_err(|error| format!("npm/lock-unavailable: {error}"))?;
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("npm/lock-invalid: {error}"))?;
    let entries = document
        .get("packages")
        .and_then(Value::as_object)
        .ok_or("npm/lock-invalid: package-lock.json has no packages object")?;
    entries
        .iter()
        .filter(|(path, _)| !path.is_empty())
        .map(|(path, value)| {
            let version = value
                .get("version")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("npm/lock-invalid: {path} has no version"))?;
            let integrity = value
                .get("integrity")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("npm/lock-invalid: {path} has no integrity"))?;
            Ok((
                path.clone(),
                LockedPackage {
                    path: path.clone(),
                    version: version.into(),
                    integrity: integrity.into(),
                },
            ))
        })
        .collect()
}

fn render_lock(
    packages: &BTreeMap<String, LockedPackage>,
    imports: &BTreeMap<String, LockedImport>,
) -> String {
    let packages = packages
        .values()
        .map(|package| {
            format!(
                "{} {{:version {} :integrity {}}}",
                quoted(&package.path),
                quoted(&package.version),
                quoted(&package.integrity)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let imports = imports
        .iter()
        .map(|(logical, import)| {
            format!(
                "{} {{:package {} :module {} :abi :{} :sha256 {} :size {} :cache {}}}",
                logical,
                quoted(&import.package),
                quoted(&import.module),
                import.abi,
                quoted(&format!("sha256:{}", import.sha256)),
                import.size,
                quoted(&import.cache)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{{:lock/format \"0.0.0-alpha\" :packages {{}} :runtime {{:rust {{:npm {{{packages}}} :imports {{{imports}}}}}}}}}\n"
    )
}

fn workspace(project: &Project) -> PathBuf {
    project.root.join("target/hara/npm/workspace")
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or("npm module path is not UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, Project) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hara-npm-wasm-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.0.0\" :project/source-paths [] :project/test-paths [] :project/extension-paths [] :project/capabilities #{} :project/runtime-profiles {:rust {:runtime/dependencies {:npm {\"raw-math\" {:version \"1.2.3\" :integrity \"sha512-AAAAAAAAAAAAAAAAAAAAAA==\"}}} :runtime/imports {Math {:package \"raw-math\" :module \"dist/math.wasm\" :abi :core.v1}}}}}",
        )
        .unwrap();
        let project = super::super::read(&root).unwrap();
        let workspace = workspace(&project);
        fs::create_dir_all(workspace.join("node_modules/raw-math/dist")).unwrap();
        fs::write(
            workspace.join("package-lock.json"),
            "{\"lockfileVersion\":3,\"packages\":{\"\":{},\"node_modules/raw-math\":{\"version\":\"1.2.3\",\"integrity\":\"sha512-AAAAAAAAAAAAAAAAAAAAAA==\"}}}",
        )
        .unwrap();
        fs::write(
            workspace.join("node_modules/raw-math/dist/math.wasm"),
            b"\0asm\x01\0\0\0",
        )
        .unwrap();
        (root, project)
    }

    #[test]
    fn locks_verified_modules_into_the_content_addressed_cache() {
        let (root, project) = fixture();
        let first = resolve_local(&project).unwrap();
        assert_eq!(first, resolve_local(&project).unwrap());
        assert!(first.contains(":runtime {:rust {:npm"));
        assert!(first.contains(":abi :core.v1"));
        assert!(first.contains(
            ":sha256 \"sha256:93a44bbb96c751218e4c00d479e4c14358122a389acca16205b1e4d0dc5f9476\""
        ));
        assert!(project
            .root
            .join("target/hara/npm/content/sha256/93a44bbb96c751218e4c00d479e4c14358122a389acca16205b1e4d0dc5f9476.wasm")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_lock_integrity_and_non_wasm_payloads() {
        let (root, project) = fixture();
        let lock = workspace(&project).join("package-lock.json");
        fs::write(
            &lock,
            "{\"lockfileVersion\":3,\"packages\":{\"\":{},\"node_modules/raw-math\":{\"version\":\"1.2.3\",\"integrity\":\"sha512-BBBBBBBBBBBBBBBBBBBBBB==\"}}}",
        )
        .unwrap();
        assert!(resolve_local(&project)
            .unwrap_err()
            .contains("npm/integrity-mismatch"));
        fs::write(
            &lock,
            "{\"lockfileVersion\":3,\"packages\":{\"\":{},\"node_modules/raw-math\":{\"version\":\"1.2.3\",\"integrity\":\"sha512-AAAAAAAAAAAAAAAAAAAAAA==\"}}}",
        )
        .unwrap();
        fs::write(
            workspace(&project).join("node_modules/raw-math/dist/math.wasm"),
            b"javascript glue",
        )
        .unwrap();
        assert!(resolve_local(&project)
            .unwrap_err()
            .contains("npm/module-invalid-media-type"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn locked_cache_installs_without_the_npm_workspace() {
        let (root, project) = fixture();
        let add = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
        fs::write(
            workspace(&project).join("node_modules/raw-math/dist/math.wasm"),
            add,
        )
        .unwrap();
        let lock = project.root.join("project.lock.edn");
        fs::write(&lock, resolve_local(&project).unwrap()).unwrap();
        fs::remove_dir_all(workspace(&project)).unwrap();

        validate_locked_cache(&project, &lock).unwrap();
        let mut runtime = Runtime::new();
        install(&project, &mut runtime).unwrap();
        assert_eq!(
            runtime
                .eval_native("(ns demo (:import Math)) (Math/add 20 22)")
                .unwrap(),
            "42"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
