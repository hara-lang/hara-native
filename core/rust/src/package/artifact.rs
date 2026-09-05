//! Generic package artifact assembly.
//!
//! The caller owns source discovery and module selection. This module only
//! turns already supplied module plans and file bytes into validated package
//! artifacts; it never opens a project tree or assigns meaning to a source
//! filename.

use crate::package_manifest::PackageManifest;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBytecode {
    pub path: String,
    pub format: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactSpec {
    pub identity: String,
    pub version: String,
    pub name: Option<String>,
    pub files: Vec<ArtifactFile>,
    pub resources: BTreeMap<String, String>,
    pub bytecode: Option<ArtifactBytecode>,
    pub extensions: String,
}

pub fn build(spec: ArtifactSpec, output: &Path) -> Result<PathBuf, String> {
    let mut files = spec.files.clone();
    if let Some(bytecode) = &spec.bytecode {
        files.push(ArtifactFile {
            path: bytecode.path.clone(),
            bytes: bytecode.bytes.clone(),
        });
    }
    let files = normalise_files(files)?;
    let file_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    for (namespace, path) in &spec.resources {
        if !file_paths.contains(path) {
            return Err(format!(
                "package resource {namespace} points to an undeclared file: {path}"
            ));
        }
    }
    if let Some(bytecode) = &spec.bytecode {
        if !file_paths.contains(&bytecode.path) {
            return Err(format!(
                "package bytecode points to an undeclared file: {}",
                bytecode.path
            ));
        }
    }

    let manifest = manifest(&spec, &files)?;
    let package_edn = PackageManifest::parse(&manifest)
        .map_err(|error| error.to_string())?
        .canonical_edn()
        .to_owned();
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let file = File::create(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut writer = ZipWriter::new(file);
    writer
        .start_file("package.edn", options)
        .map_err(zip_error)?;
    writer.write_all(package_edn.as_bytes()).map_err(io_error)?;
    for file in files {
        writer.start_file(&file.path, options).map_err(zip_error)?;
        writer.write_all(&file.bytes).map_err(io_error)?;
    }
    writer.finish().map_err(zip_error)?;
    PackageManifest::read_archive(output).map_err(|error| error.to_string())?;
    Ok(output.to_path_buf())
}

pub(super) fn inspect(path: &Path) -> Result<String, String> {
    PackageManifest::read_archive(path)
        .map(|manifest| manifest.canonical_edn().to_owned())
        .map_err(|error| error.to_string())
}

fn normalise_files(files: Vec<ArtifactFile>) -> Result<Vec<ArtifactFile>, String> {
    let mut by_path = BTreeMap::new();
    for file in files {
        let path = normalise_path(&file.path)?;
        if by_path.insert(path.clone(), file.bytes).is_some() {
            return Err(format!("duplicate package archive path: {path}"));
        }
    }
    Ok(by_path
        .into_iter()
        .map(|(path, bytes)| ArtifactFile { path, bytes })
        .collect())
}

fn manifest(spec: &ArtifactSpec, files: &[ArtifactFile]) -> Result<String, String> {
    if spec.identity.is_empty() || spec.version.is_empty() {
        return Err("package artifact requires non-empty identity and version".into());
    }
    let mut tree = Sha256::new();
    let mut declarations = String::new();
    for file in files {
        let digest = Sha256::digest(&file.bytes);
        tree.update(file.path.as_bytes());
        tree.update([0]);
        tree.update(&file.bytes);
        declarations.push_str(&format!(
            " {} {{:sha256 \"sha256:{}\" :size {}}}",
            edn_string(&file.path),
            hex(&digest),
            file.bytes.len()
        ));
    }
    let resources = spec
        .resources
        .iter()
        .map(|(namespace, path)| format!(" {} {}", edn_string(namespace), edn_string(path)))
        .collect::<String>();
    let name = spec
        .name
        .as_deref()
        .map(|value| format!(" :name {}", edn_string(value)))
        .unwrap_or_default();
    let bytecode = spec
        .bytecode
        .as_ref()
        .map(|value| {
            format!(
                " :bytecode {{:format {} :path {} :sha256 \"sha256:{}\"}}",
                edn_string(&value.format),
                edn_string(&value.path),
                hex(&Sha256::digest(&value.bytes))
            )
        })
        .unwrap_or_default();
    let extensions = if spec.extensions.is_empty() {
        "{}".to_owned()
    } else {
        spec.extensions.clone()
    };
    Ok(format!(
        "{{:harp/format \"0.0.0-alpha\" :package {{:identity {}{} :version {}}} :files {{{}}} :resources {{{}}} :extensions {}{} :integrity {{:tree-sha256 \"sha256:{}\"}}}}",
        edn_string(&spec.identity),
        name,
        edn_string(&spec.version),
        declarations,
        resources,
        extensions,
        bytecode,
        hex(&tree.finalize())
    ))
}

fn normalise_path(value: &str) -> Result<String, String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe package archive path: {value}"));
    }
    Ok(value.replace('\\', "/"))
}

pub(super) fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err(format!("unsafe package path: {}", path.display()));
    }
    Ok(())
}

fn edn_string(value: &str) -> String {
    crate::kernel::Form::String(value.to_owned()).to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

fn zip_error(error: zip::result::ZipError) -> String {
    error.to_string()
}
