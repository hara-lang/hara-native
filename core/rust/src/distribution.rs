//! Relocatable source-package distributions.
//!
//! A distribution keeps the generic native host and canonical HAL separate:
//! the copied host sits in `bin/`, while the verified HARP archive and its
//! declarative launch contract sit in `lib/` next to it.

use crate::kernel::{parse_forms, Form};
use crate::package;
use crate::package_manifest::PackageManifest;
use crate::project;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const FORMAT: &str = "hara-distribution/v1";
pub const ARCHIVE_PATH: &str = "lib/hara.harp";
pub const MANIFEST_PATH: &str = "lib/release.edn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub launcher: String,
    pub entry: String,
    pub archive: PathBuf,
    pub archive_sha256: String,
    pub source_identity: String,
    pub source_version: String,
    pub native_version: String,
    pub native_sha256: String,
}

/// Builds a directory that can be relocated as one unit. The caller supplies
/// the native executable that is copied as the launcher; the package is built
/// from the declared project and verified again before this function succeeds.
pub fn build(project_path: &Path, native_binary: &Path, output: &Path) -> Result<Manifest, String> {
    let project = project::read(project_path)?;
    let declaration = project.distribution.as_ref().ok_or_else(|| {
        "project.edn :project/distribution is required for distribution build".to_owned()
    })?;
    ensure_empty_output(output)?;
    if !native_binary.is_file() {
        return Err(format!(
            "distribution native binary is not a regular file: {}",
            native_binary.display()
        ));
    }

    let archive = output.join(ARCHIVE_PATH);
    let launcher = launcher_path(output, &declaration.launcher);
    let archive_parent = archive
        .parent()
        .ok_or_else(|| "distribution archive path has no parent".to_owned())?;
    let launcher_parent = launcher
        .parent()
        .ok_or_else(|| "distribution launcher path has no parent".to_owned())?;
    fs::create_dir_all(archive_parent).map_err(io_error)?;
    fs::create_dir_all(launcher_parent).map_err(io_error)?;
    package::build_path(&project.root, Some(&archive))?;
    fs::copy(native_binary, &launcher).map_err(io_error)?;

    let package = PackageManifest::read_archive(&archive).map_err(|error| error.to_string())?;
    let manifest = Manifest {
        launcher: declaration.launcher.clone(),
        entry: declaration.entry.clone(),
        archive: PathBuf::from(ARCHIVE_PATH),
        archive_sha256: checksum(&archive)?,
        source_identity: package.identity,
        source_version: package.version.to_string(),
        native_version: env!("CARGO_PKG_VERSION").into(),
        native_sha256: checksum(&launcher)?,
    };
    let manifest_path = output.join(MANIFEST_PATH);
    fs::write(&manifest_path, format!("{}\n", manifest.to_edn())).map_err(io_error)?;
    verify(output, &launcher)?;
    Ok(manifest)
}

/// Reads and verifies the local companion contract before a host loads any
/// source from its HARP archive. A release archive's signature authenticates
/// `release.edn`; this function enforces the exact digests it records.
pub fn verify(root: &Path, native_binary: &Path) -> Result<Manifest, String> {
    let manifest = read(root)?;
    let expected_launcher = launcher_path(root, &manifest.launcher);
    if native_binary != expected_launcher {
        return Err(format!(
            "distribution launcher path mismatch: expected {}, received {}",
            expected_launcher.display(),
            native_binary.display()
        ));
    }
    check_checksum(native_binary, &manifest.native_sha256, "native launcher")?;
    if manifest.native_version != env!("CARGO_PKG_VERSION") {
        return Err(format!(
            "distribution native version mismatch: manifest {}, launcher {}",
            manifest.native_version,
            env!("CARGO_PKG_VERSION")
        ));
    }
    let archive = root.join(&manifest.archive);
    check_checksum(&archive, &manifest.archive_sha256, "source archive")?;
    let package = PackageManifest::read_archive(&archive).map_err(|error| error.to_string())?;
    if package.identity != manifest.source_identity
        || package.version.to_string() != manifest.source_version
    {
        return Err(format!(
            "distribution source package mismatch: manifest {} {}, archive {} {}",
            manifest.source_identity, manifest.source_version, package.identity, package.version
        ));
    }
    Ok(manifest)
}

pub fn read(root: &Path) -> Result<Manifest, String> {
    let path = root.join(MANIFEST_PATH);
    let source = fs::read_to_string(&path).map_err(|error| {
        format!(
            "cannot read distribution manifest {}: {error}",
            path.display()
        )
    })?;
    let forms = parse_forms(&source)?;
    let [Form::Map(entries)] = forms.as_slice() else {
        return Err("distribution manifest must contain one EDN map".into());
    };
    let format = string_field(entries, "distribution/format")?;
    if format != FORMAT {
        return Err(format!(
            "unsupported distribution manifest format: {format}"
        ));
    }
    let launcher = string_field(entries, "launcher")?;
    if !valid_launcher(&launcher) {
        return Err("distribution manifest launcher is invalid".into());
    }
    let entry = symbol_field(entries, "entry")?;
    if !valid_entry(&entry) {
        return Err("distribution manifest entry must name namespace/symbol".into());
    }
    let archive = relative_path(
        &string_field(entries, "archive")?,
        "distribution manifest archive",
    )?;
    let archive_sha256 = checksum_value(&string_field(entries, "archive/sha256")?)?;
    let source_identity = string_field(entries, "source/identity")?;
    let source_version = string_field(entries, "source/version")?;
    let native_version = string_field(entries, "native/version")?;
    let native_sha256 = checksum_value(&string_field(entries, "native/sha256")?)?;
    Ok(Manifest {
        launcher,
        entry,
        archive,
        archive_sha256,
        source_identity,
        source_version,
        native_version,
        native_sha256,
    })
}

impl Manifest {
    pub fn to_edn(&self) -> String {
        Form::Map(vec![
            (
                Form::Keyword("distribution/format".into()),
                Form::String(FORMAT.into()),
            ),
            (
                Form::Keyword("launcher".into()),
                Form::String(self.launcher.clone()),
            ),
            (
                Form::Keyword("entry".into()),
                Form::Symbol(self.entry.clone()),
            ),
            (
                Form::Keyword("archive".into()),
                Form::String(self.archive.to_string_lossy().into_owned()),
            ),
            (
                Form::Keyword("archive/sha256".into()),
                Form::String(self.archive_sha256.clone()),
            ),
            (
                Form::Keyword("source/identity".into()),
                Form::String(self.source_identity.clone()),
            ),
            (
                Form::Keyword("source/version".into()),
                Form::String(self.source_version.clone()),
            ),
            (
                Form::Keyword("native/version".into()),
                Form::String(self.native_version.clone()),
            ),
            (
                Form::Keyword("native/sha256".into()),
                Form::String(self.native_sha256.clone()),
            ),
        ])
        .to_string()
    }
}

fn ensure_empty_output(output: &Path) -> Result<(), String> {
    if output.exists() {
        let mut entries = fs::read_dir(output).map_err(io_error)?;
        if entries.next().is_some() {
            return Err(format!(
                "distribution output already exists and is not empty: {}",
                output.display()
            ));
        }
    } else {
        fs::create_dir_all(output).map_err(io_error)?;
    }
    Ok(())
}

fn launcher_path(root: &Path, launcher: &str) -> PathBuf {
    let name = if cfg!(windows) {
        format!("{launcher}.exe")
    } else {
        launcher.into()
    };
    root.join("bin").join(name)
}

fn string_field(entries: &[(Form, Form)], key: &str) -> Result<String, String> {
    match field(entries, key) {
        Some(Form::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("distribution manifest :{key} must be a string")),
        None => Err(format!("distribution manifest is missing :{key}")),
    }
}

fn symbol_field(entries: &[(Form, Form)], key: &str) -> Result<String, String> {
    match field(entries, key) {
        Some(Form::Symbol(value)) => Ok(value.clone()),
        Some(_) => Err(format!("distribution manifest :{key} must be a symbol")),
        None => Err(format!("distribution manifest is missing :{key}")),
    }
}

fn field<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(candidate) if candidate == key => Some(value),
            _ => None,
        })
}

fn valid_launcher(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-')
}

fn valid_entry(value: &str) -> bool {
    value.matches('/').count() == 1
        && value
            .split_once('/')
            .is_some_and(|(namespace, symbol)| !namespace.is_empty() && !symbol.is_empty())
}

fn relative_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("{label} must be a non-empty relative path"));
    }
    Ok(path)
}

fn checksum(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(io_error)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn checksum_value(value: &str) -> Result<String, String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err("distribution checksum must use sha256:<lowercase-hex>".into());
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("distribution checksum must use sha256:<lowercase-hex>".into());
    }
    Ok(value.into())
}

fn check_checksum(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let actual = checksum(path)?;
    if actual != expected {
        return Err(format!(
            "distribution {label} digest mismatch: expected {expected}, received {actual}"
        ));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::{build, read, verify, ARCHIVE_PATH, MANIFEST_PATH};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hara-distribution-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn fixture(root: &std::path::Path) {
        fs::create_dir_all(root.join("src/demo")).unwrap();
        fs::write(
            root.join("project.edn"),
            "{:hara/type :project :hara/version \"1.0.0\" :project/id demo/app :project/version \"1.2.3\" :project/source-paths [\"src\"] :project/test-paths [] :project/extension-paths [] :project/main demo.cli :project/distribution {:launcher \"hara\" :entry demo.cli/main} :project/capabilities #{}}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/demo/cli.hal"),
            "(ns demo.cli)\n(defn main [argv] argv)\n",
        )
        .unwrap();
    }

    #[test]
    fn builds_and_verifies_a_relocatable_source_distribution() {
        let root = temp("build");
        let output = root.join("output");
        let native = root.join("native-host");
        fixture(&root);
        fs::write(&native, "native-host").unwrap();

        let manifest = build(&root, &native, &output).unwrap();
        assert_eq!(manifest.launcher, "hara");
        assert_eq!(manifest.entry, "demo.cli/main");
        assert!(output.join(ARCHIVE_PATH).is_file());
        assert!(output.join(MANIFEST_PATH).is_file());
        assert_eq!(read(&output).unwrap(), manifest);
        assert_eq!(verify(&output, &output.join("bin/hara")).unwrap(), manifest);

        fs::write(output.join(ARCHIVE_PATH), "modified").unwrap();
        assert!(verify(&output, &output.join("bin/hara"))
            .unwrap_err()
            .contains("source archive digest mismatch"));
        fs::remove_dir_all(root).unwrap();
    }
}
