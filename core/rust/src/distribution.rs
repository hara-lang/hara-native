//! Relocatable native distributions.
//!
//! A distribution keeps the generic native host and canonical HAL separate:
//! the copied host sits in `bin/`, while the verified HARP artifact and its
//! declarative launch contract sit in `lib/` next to it. Source discovery and
//! compilation are owned by the caller before this boundary.

use crate::kernel::{parse_forms, Form};
use crate::package;
use crate::package_manifest::PackageManifest;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

pub const FORMAT: &str = "hara-distribution/v1";
pub const ARCHIVE_PATH: &str = "lib/hara.harp";
pub const MANIFEST_PATH: &str = "lib/release.edn";

/// A self-contained native executable. The native loader treats the final
/// fixed-size footer as an opt-in marker, so ordinary platform executables
/// remain valid hosts without a Hara payload.
pub const SEALED_FORMAT: &str = "hara-executable/v1";
const SEALED_MAGIC: &[u8; 8] = b"HARAEXE1";
const SEALED_VERSION: u32 = 1;
const SEALED_FOOTER_BYTES: usize = 96;
const SEALED_PAYLOAD_HEADER_BYTES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealArchive {
    pub path: PathBuf,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealSpec {
    pub host: PathBuf,
    pub output: PathBuf,
    pub entry: String,
    pub archives: Vec<SealArchive>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedArchive {
    pub identity: String,
    pub version: String,
    pub sha256: String,
    /// Offset from the first payload byte, never from the beginning of the
    /// executable. This makes the descriptor independent of host location.
    pub offset: u64,
    pub length: u64,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedManifest {
    pub entry: String,
    pub archives: Vec<SealedArchive>,
    pub host_sha256: String,
    pub payload_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedInstallation {
    pub manifest: SealedManifest,
    pub roots: Vec<PathBuf>,
    pub primary: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SealedFooter {
    payload_start: usize,
    payload_length: usize,
    host_sha256: [u8; 32],
    payload_sha256: [u8; 32],
}

/// Writes a single executable that contains a verified native host and an
/// ordered set of canonical HARP archives. The archive marked `:primary` owns
/// the entry point; all other archives are installed first so its project lock
/// can resolve them from the normal content-addressed package cache.
pub fn seal(spec: &SealSpec) -> Result<SealedManifest, String> {
    validate_seal_spec(spec)?;
    if spec.output.exists() {
        return Err(format!(
            "sealed executable output already exists: {}; choose a new output path",
            spec.output.display()
        ));
    }

    let source_host_permissions = fs::metadata(&spec.host)
        .map_err(|error| {
            format!(
                "cannot inspect sealed executable host {}: {error}",
                spec.host.display()
            )
        })?
        .permissions();
    let source_host = fs::read(&spec.host).map_err(|error| {
        format!(
            "cannot read sealed executable host {}: {error}",
            spec.host.display()
        )
    })?;
    let host = match parse_sealed_bytes(&source_host)? {
        Some((_, footer)) => source_host[..footer.payload_start].to_vec(),
        None => source_host,
    };
    if host.is_empty() {
        return Err("sealed executable host has no native bytes".into());
    }

    let mut identities = HashSet::new();
    let mut archives = Vec::with_capacity(spec.archives.len());
    let mut contents = Vec::with_capacity(spec.archives.len());
    for archive in &spec.archives {
        let manifest = PackageManifest::read_archive(&archive.path).map_err(|error| {
            format!(
                "cannot seal HARP archive {}: {error}",
                archive.path.display()
            )
        })?;
        if !identities.insert(manifest.identity.clone()) {
            return Err(format!(
                "sealed executable declares duplicate package identity: {}",
                manifest.identity
            ));
        }
        let bytes = fs::read(&archive.path).map_err(|error| {
            format!(
                "cannot read sealed HARP archive {}: {error}",
                archive.path.display()
            )
        })?;
        archives.push(SealedArchive {
            identity: manifest.identity,
            version: manifest.version.to_string(),
            sha256: checksum_bytes(&bytes),
            offset: 0,
            length: u64::try_from(bytes.len())
                .map_err(|_| "sealed archive length exceeds u64".to_owned())?,
            primary: archive.primary,
        });
        contents.push(bytes);
    }

    let descriptor = sealed_descriptor_fixed_point(&spec.entry, &mut archives)?;
    let mut payload = Vec::with_capacity(
        SEALED_PAYLOAD_HEADER_BYTES
            .checked_add(descriptor.len())
            .and_then(|value| {
                contents
                    .iter()
                    .try_fold(value, |total, bytes| total.checked_add(bytes.len()))
            })
            .ok_or("sealed executable payload is too large")?,
    );
    payload.extend_from_slice(
        &u64::try_from(descriptor.len())
            .map_err(|_| "sealed executable descriptor is too large")?
            .to_be_bytes(),
    );
    payload.extend_from_slice(&descriptor);
    for bytes in &contents {
        payload.extend_from_slice(bytes);
    }

    let host_sha256 = checksum_bytes(&host);
    let payload_sha256 = checksum_bytes(&payload);
    let manifest = SealedManifest {
        entry: spec.entry.clone(),
        archives,
        host_sha256: host_sha256.clone(),
        payload_sha256: payload_sha256.clone(),
    };
    let footer = sealed_footer(
        host.len(),
        payload.len(),
        &checksum_digest(&host),
        &checksum_digest(&payload),
    )?;
    write_sealed_atomically(
        &spec.output,
        &host,
        &payload,
        &footer,
        source_host_permissions,
    )?;
    Ok(manifest)
}

/// Returns `None` for an ordinary native executable. A footer with the Hara
/// magic is never ignored: malformed or tampered sealed binaries fail closed.
pub fn inspect_sealed(path: &Path) -> Result<Option<SealedManifest>, String> {
    if !has_sealed_footer(path)? {
        return Ok(None);
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read sealed executable {}: {error}", path.display()))?;
    parse_sealed_bytes(&bytes).map(|found| found.map(|(manifest, _)| manifest))
}

/// Revalidates a sealed executable's envelope and every embedded archive's
/// package manifest. It does not mutate the package cache.
pub fn verify_sealed(path: &Path) -> Result<Option<SealedManifest>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read sealed executable {}: {error}", path.display()))?;
    let Some((manifest, footer)) = parse_sealed_bytes(&bytes)? else {
        return Ok(None);
    };
    let payload_end = footer
        .payload_start
        .checked_add(footer.payload_length)
        .ok_or("sealed executable payload range overflows")?;
    let payload = &bytes[footer.payload_start..payload_end];
    for (index, archive) in manifest.archives.iter().enumerate() {
        let path = temporary_archive_path(archive, index);
        write_temporary_archive(&path, archive_bytes(payload, archive)?)?;
        let checked = (|| {
            let package =
                PackageManifest::read_archive(&path).map_err(|error| error.to_string())?;
            verify_embedded_package(archive, &package)
        })();
        let _ = fs::remove_file(&path);
        checked?;
    }
    Ok(Some(manifest))
}

/// Installs a sealed executable's archives into the normal content-addressed
/// package cache. The only temporary HARP copies are deleted on every path;
/// the executable itself has no adjacent package files.
pub fn install_sealed(path: &Path) -> Result<Option<SealedInstallation>, String> {
    install_sealed_at(path, &package::install_root())
}

/// Installs a sealed executable into an explicit package-cache root.
///
/// A launcher uses a payload-specific root to preserve the immutability of
/// installed semantic-version registrations across independently rebuilt
/// executable payloads.
pub fn install_sealed_at(
    path: &Path,
    distribution_root: &Path,
) -> Result<Option<SealedInstallation>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot read sealed executable {}: {error}", path.display()))?;
    let Some((manifest, footer)) = parse_sealed_bytes(&bytes)? else {
        return Ok(None);
    };
    let payload_end = footer
        .payload_start
        .checked_add(footer.payload_length)
        .ok_or("sealed executable payload range overflows")?;
    let payload = &bytes[footer.payload_start..payload_end];
    let mut roots = Vec::with_capacity(manifest.archives.len());
    let mut primary = None;
    for (index, archive) in manifest.archives.iter().enumerate() {
        let temporary = temporary_archive_path(archive, index);
        write_temporary_archive(&temporary, archive_bytes(payload, archive)?)?;
        let installed = (|| {
            let package =
                PackageManifest::read_archive(&temporary).map_err(|error| error.to_string())?;
            verify_embedded_package(archive, &package)?;
            package::install_path_at(&temporary, distribution_root)
        })();
        let _ = fs::remove_file(&temporary);
        let installed = installed?;
        if archive.primary {
            primary = Some(installed.clone());
        }
        roots.push(installed);
    }
    Ok(Some(SealedInstallation {
        manifest,
        roots,
        primary: primary.expect("validated sealed manifest has one primary archive"),
    }))
}

fn validate_seal_spec(spec: &SealSpec) -> Result<(), String> {
    if !spec.host.is_file() {
        return Err(format!(
            "sealed executable host is not a regular file: {}",
            spec.host.display()
        ));
    }
    if !valid_entry(&spec.entry) {
        return Err("sealed executable entry must name namespace/symbol".into());
    }
    if spec.archives.is_empty() {
        return Err("sealed executable requires at least one HARP archive".into());
    }
    let primary = spec
        .archives
        .iter()
        .filter(|archive| archive.primary)
        .count();
    if primary != 1 {
        return Err("sealed executable requires exactly one primary HARP archive".into());
    }
    if spec.archives.iter().any(|archive| !archive.path.is_file()) {
        return Err("sealed executable archives must be regular files".into());
    }
    Ok(())
}

fn sealed_descriptor_fixed_point(
    entry: &str,
    archives: &mut [SealedArchive],
) -> Result<Vec<u8>, String> {
    let mut descriptor_length = 0usize;
    for _ in 0..16 {
        let mut offset = SEALED_PAYLOAD_HEADER_BYTES
            .checked_add(descriptor_length)
            .ok_or("sealed executable descriptor is too large")?;
        for archive in archives.iter_mut() {
            archive.offset = u64::try_from(offset)
                .map_err(|_| "sealed executable offset exceeds u64".to_owned())?;
            offset = offset
                .checked_add(usize::try_from(archive.length).map_err(|_| {
                    "sealed executable archive length does not fit this platform".to_owned()
                })?)
                .ok_or("sealed executable payload is too large")?;
        }
        let descriptor = sealed_descriptor(entry, archives)?;
        if descriptor.len() == descriptor_length {
            return Ok(descriptor);
        }
        descriptor_length = descriptor.len();
    }
    Err("sealed executable descriptor offsets did not converge".into())
}

fn sealed_descriptor(entry: &str, archives: &[SealedArchive]) -> Result<Vec<u8>, String> {
    let archives = archives
        .iter()
        .map(|archive| {
            Ok(Form::Map(vec![
                (
                    Form::Keyword("identity".into()),
                    Form::String(archive.identity.clone()),
                ),
                (
                    Form::Keyword("version".into()),
                    Form::String(archive.version.clone()),
                ),
                (
                    Form::Keyword("sha256".into()),
                    Form::String(archive.sha256.clone()),
                ),
                (
                    Form::Keyword("offset".into()),
                    Form::Number(
                        i64::try_from(archive.offset)
                            .map_err(|_| "sealed executable offset exceeds i64")?,
                    ),
                ),
                (
                    Form::Keyword("length".into()),
                    Form::Number(
                        i64::try_from(archive.length)
                            .map_err(|_| "sealed executable length exceeds i64")?,
                    ),
                ),
                (Form::Keyword("primary".into()), Form::Bool(archive.primary)),
            ]))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Form::Map(vec![
        (
            Form::Keyword("executable/format".into()),
            Form::String(SEALED_FORMAT.into()),
        ),
        (Form::Keyword("entry".into()), Form::Symbol(entry.into())),
        (Form::Keyword("archives".into()), Form::Vector(archives)),
    ])
    .to_string()
    .into_bytes())
}

fn sealed_footer(
    payload_start: usize,
    payload_length: usize,
    host_sha256: &[u8; 32],
    payload_sha256: &[u8; 32],
) -> Result<[u8; SEALED_FOOTER_BYTES], String> {
    let mut footer = [0_u8; SEALED_FOOTER_BYTES];
    footer[0..8].copy_from_slice(SEALED_MAGIC);
    footer[8..12].copy_from_slice(&SEALED_VERSION.to_be_bytes());
    footer[16..24].copy_from_slice(
        &u64::try_from(payload_start)
            .map_err(|_| "sealed executable payload start exceeds u64")?
            .to_be_bytes(),
    );
    footer[24..32].copy_from_slice(
        &u64::try_from(payload_length)
            .map_err(|_| "sealed executable payload length exceeds u64")?
            .to_be_bytes(),
    );
    footer[32..64].copy_from_slice(host_sha256);
    footer[64..96].copy_from_slice(payload_sha256);
    Ok(footer)
}

fn write_sealed_atomically(
    output: &Path,
    host: &[u8],
    payload: &[u8],
    footer: &[u8; SEALED_FOOTER_BYTES],
    permissions: fs::Permissions,
) -> Result<(), String> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or("sealed executable output has no parent directory")?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("sealed executable output must have a UTF-8 filename")?;
    let temporary = parent.join(format!(".{name}.seal-{}", std::process::id()));
    let result = (|| {
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(host).map_err(io_error)?;
        file.write_all(payload).map_err(io_error)?;
        file.write_all(footer).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::set_permissions(&temporary, permissions).map_err(io_error)?;
        fs::rename(&temporary, output).map_err(io_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn parse_sealed_bytes(bytes: &[u8]) -> Result<Option<(SealedManifest, SealedFooter)>, String> {
    let Some(footer) = parse_sealed_footer(bytes)? else {
        return Ok(None);
    };
    let payload_end = footer
        .payload_start
        .checked_add(footer.payload_length)
        .ok_or("sealed executable payload range overflows")?;
    let payload = &bytes[footer.payload_start..payload_end];
    if checksum_digest(&bytes[..footer.payload_start]) != footer.host_sha256 {
        return Err("sealed executable host digest mismatch".into());
    }
    if checksum_digest(payload) != footer.payload_sha256 {
        return Err("sealed executable payload digest mismatch".into());
    }
    if payload.len() < SEALED_PAYLOAD_HEADER_BYTES {
        return Err("sealed executable payload is missing its descriptor length".into());
    }
    let descriptor_length = usize::try_from(u64::from_be_bytes(
        payload[0..SEALED_PAYLOAD_HEADER_BYTES]
            .try_into()
            .expect("fixed sealed descriptor header length"),
    ))
    .map_err(|_| "sealed executable descriptor length exceeds this platform")?;
    let descriptor_end = SEALED_PAYLOAD_HEADER_BYTES
        .checked_add(descriptor_length)
        .ok_or("sealed executable descriptor range overflows")?;
    if descriptor_end > payload.len() {
        return Err("sealed executable descriptor exceeds its payload".into());
    }
    let descriptor = std::str::from_utf8(&payload[SEALED_PAYLOAD_HEADER_BYTES..descriptor_end])
        .map_err(|_| "sealed executable descriptor is not UTF-8")?;
    let forms = parse_forms(descriptor)?;
    let [Form::Map(entries)] = forms.as_slice() else {
        return Err("sealed executable descriptor must contain one EDN map".into());
    };
    let mut manifest = sealed_manifest_from_entries(entries)?;
    manifest.host_sha256 = checksum_bytes(&bytes[..footer.payload_start]);
    manifest.payload_sha256 = checksum_bytes(payload);
    validate_sealed_archives(payload, descriptor_end, &manifest.archives)?;
    Ok(Some((manifest, footer)))
}

fn parse_sealed_footer(bytes: &[u8]) -> Result<Option<SealedFooter>, String> {
    if bytes.len() < SEALED_FOOTER_BYTES {
        return Ok(None);
    }
    let start = bytes.len() - SEALED_FOOTER_BYTES;
    let footer = &bytes[start..];
    if &footer[0..8] != SEALED_MAGIC {
        return Ok(None);
    }
    let version = u32::from_be_bytes(footer[8..12].try_into().expect("fixed footer version"));
    if version != SEALED_VERSION {
        return Err(format!("unsupported sealed executable version: {version}"));
    }
    if footer[12..16] != [0, 0, 0, 0] {
        return Err("sealed executable footer reserved bytes must be zero".into());
    }
    let payload_start = usize::try_from(u64::from_be_bytes(
        footer[16..24]
            .try_into()
            .expect("fixed footer payload start"),
    ))
    .map_err(|_| "sealed executable payload start exceeds this platform")?;
    let payload_length = usize::try_from(u64::from_be_bytes(
        footer[24..32]
            .try_into()
            .expect("fixed footer payload length"),
    ))
    .map_err(|_| "sealed executable payload length exceeds this platform")?;
    let expected_payload_end = bytes
        .len()
        .checked_sub(SEALED_FOOTER_BYTES)
        .ok_or("sealed executable footer is truncated")?;
    if payload_start
        .checked_add(payload_length)
        .filter(|end| *end == expected_payload_end)
        .is_none()
    {
        return Err("sealed executable payload range does not end before its footer".into());
    }
    let mut host_sha256 = [0_u8; 32];
    host_sha256.copy_from_slice(&footer[32..64]);
    let mut payload_sha256 = [0_u8; 32];
    payload_sha256.copy_from_slice(&footer[64..96]);
    Ok(Some(SealedFooter {
        payload_start,
        payload_length,
        host_sha256,
        payload_sha256,
    }))
}

fn sealed_manifest_from_entries(entries: &[(Form, Form)]) -> Result<SealedManifest, String> {
    let format = string_field(entries, "executable/format")?;
    if format != SEALED_FORMAT {
        return Err(format!("unsupported sealed executable format: {format}"));
    }
    let entry = symbol_field(entries, "entry")?;
    if !valid_entry(&entry) {
        return Err("sealed executable entry must name namespace/symbol".into());
    }
    let values = match field(entries, "archives") {
        Some(Form::Vector(values)) if !values.is_empty() => values,
        Some(Form::Vector(_)) => return Err("sealed executable has no archives".into()),
        Some(_) => return Err("sealed executable :archives must be a vector".into()),
        None => return Err("sealed executable is missing :archives".into()),
    };
    let archives = values
        .iter()
        .map(sealed_archive_from_form)
        .collect::<Result<Vec<_>, _>>()?;
    let primary = archives.iter().filter(|archive| archive.primary).count();
    if primary != 1 {
        return Err("sealed executable requires exactly one primary archive".into());
    }
    let mut identities = HashSet::new();
    for archive in &archives {
        if !identities.insert(archive.identity.clone()) {
            return Err(format!(
                "sealed executable declares duplicate package identity: {}",
                archive.identity
            ));
        }
    }
    Ok(SealedManifest {
        entry,
        archives,
        host_sha256: String::new(),
        payload_sha256: String::new(),
    })
}

fn sealed_archive_from_form(value: &Form) -> Result<SealedArchive, String> {
    let Form::Map(entries) = value else {
        return Err("sealed executable archive must be an EDN map".into());
    };
    let identity = string_field(entries, "identity")?;
    let version = string_field(entries, "version")?;
    let sha256 = checksum_value(&string_field(entries, "sha256")?)?;
    let offset = non_negative_number_field(entries, "offset")?;
    let length = non_negative_number_field(entries, "length")?;
    if length == 0 {
        return Err("sealed executable archive length must be positive".into());
    }
    let primary = match field(entries, "primary") {
        Some(Form::Bool(value)) => *value,
        Some(_) => return Err("sealed executable archive :primary must be boolean".into()),
        None => return Err("sealed executable archive is missing :primary".into()),
    };
    Ok(SealedArchive {
        identity,
        version,
        sha256,
        offset,
        length,
        primary,
    })
}

fn non_negative_number_field(entries: &[(Form, Form)], key: &str) -> Result<u64, String> {
    match field(entries, key) {
        Some(Form::Number(value)) if *value >= 0 => Ok(*value as u64),
        Some(Form::Number(_)) => Err(format!("sealed executable :{key} must be non-negative")),
        Some(_) => Err(format!("sealed executable :{key} must be an integer")),
        None => Err(format!("sealed executable archive is missing :{key}")),
    }
}

fn validate_sealed_archives(
    payload: &[u8],
    descriptor_end: usize,
    archives: &[SealedArchive],
) -> Result<(), String> {
    let mut previous_end = descriptor_end;
    for archive in archives {
        let offset = usize::try_from(archive.offset)
            .map_err(|_| "sealed executable archive offset exceeds this platform")?;
        let length = usize::try_from(archive.length)
            .map_err(|_| "sealed executable archive length exceeds this platform")?;
        if offset != previous_end {
            return Err("sealed executable archives must be contiguous and ordered".into());
        }
        let end = offset
            .checked_add(length)
            .ok_or("sealed executable archive range overflows")?;
        if end > payload.len() {
            return Err("sealed executable archive exceeds its payload".into());
        }
        if checksum_bytes(&payload[offset..end]) != archive.sha256 {
            return Err(format!(
                "sealed executable archive digest mismatch: {}",
                archive.identity
            ));
        }
        previous_end = end;
    }
    if previous_end != payload.len() {
        return Err("sealed executable payload has unclaimed trailing bytes".into());
    }
    Ok(())
}

fn archive_bytes<'a>(payload: &'a [u8], archive: &SealedArchive) -> Result<&'a [u8], String> {
    let offset = usize::try_from(archive.offset)
        .map_err(|_| "sealed executable archive offset exceeds this platform")?;
    let length = usize::try_from(archive.length)
        .map_err(|_| "sealed executable archive length exceeds this platform")?;
    let end = offset
        .checked_add(length)
        .ok_or("sealed executable archive range overflows")?;
    payload
        .get(offset..end)
        .ok_or_else(|| "sealed executable archive exceeds its payload".into())
}

fn temporary_archive_path(archive: &SealedArchive, index: usize) -> PathBuf {
    let digest = archive.sha256.trim_start_matches("sha256:");
    std::env::temp_dir().join(format!(
        "hara-sealed-{}-{index}-{}.harp",
        std::process::id(),
        &digest[..12]
    ))
}

fn write_temporary_archive(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "cannot create sealed archive temporary {}: {error}",
                path.display()
            )
        })?;
    let result = file.write_all(bytes).map_err(io_error);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn verify_embedded_package(
    archive: &SealedArchive,
    package: &PackageManifest,
) -> Result<(), String> {
    if package.identity != archive.identity || package.version.to_string() != archive.version {
        return Err(format!(
            "sealed executable archive identity mismatch: expected {} {}, received {} {}",
            archive.identity, archive.version, package.identity, package.version
        ));
    }
    Ok(())
}

fn checksum_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn checksum_bytes(bytes: &[u8]) -> String {
    let digest = checksum_digest(bytes);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

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
/// the already-built HARP archive, launcher metadata, and native executable;
/// Native only copies and verifies those artifacts.
pub fn build(
    archive: &Path,
    native_binary: &Path,
    output: &Path,
    launcher: &str,
    entry: &str,
) -> Result<Manifest, String> {
    build_with_options(archive, native_binary, output, launcher, entry, false)
}

/// Rebuilds a known, integrity-checked Hara distribution in place.
///
/// An unrelated non-empty directory is never removed. This lets a project
/// refresh its generated companion launcher without making the output path a
/// broad deletion capability.
pub fn build_replace(
    archive: &Path,
    native_binary: &Path,
    output: &Path,
    launcher: &str,
    entry: &str,
) -> Result<Manifest, String> {
    build_with_options(archive, native_binary, output, launcher, entry, true)
}

fn build_with_options(
    archive_source: &Path,
    native_binary: &Path,
    output: &Path,
    launcher_name: &str,
    entry_name: &str,
    replace: bool,
) -> Result<Manifest, String> {
    validate_output(output, replace)?;
    if !valid_launcher(launcher_name) {
        return Err(
            "distribution launcher must contain lowercase letters, digits, or hyphens".into(),
        );
    }
    if !valid_entry(entry_name) {
        return Err("distribution entry must name namespace/symbol".into());
    }
    if !archive_source.is_file() {
        return Err(format!(
            "distribution HARP archive is not a regular file: {}",
            archive_source.display()
        ));
    }
    if !native_binary.is_file() {
        return Err(format!(
            "distribution native binary is not a regular file: {}",
            native_binary.display()
        ));
    }

    let staging = create_staging_output(output)?;
    let archive = staging.join(ARCHIVE_PATH);
    let launcher = launcher_path(&staging, launcher_name);
    let archive_parent = archive
        .parent()
        .ok_or_else(|| "distribution archive path has no parent".to_owned())?;
    let launcher_parent = launcher
        .parent()
        .ok_or_else(|| "distribution launcher path has no parent".to_owned())?;
    let build = (|| {
        fs::create_dir_all(archive_parent).map_err(io_error)?;
        fs::create_dir_all(launcher_parent).map_err(io_error)?;
        fs::copy(archive_source, &archive).map_err(io_error)?;
        fs::copy(native_binary, &launcher).map_err(io_error)?;

        let package = PackageManifest::read_archive(&archive).map_err(|error| error.to_string())?;
        let manifest = Manifest {
            launcher: launcher_name.into(),
            entry: entry_name.into(),
            archive: PathBuf::from(ARCHIVE_PATH),
            archive_sha256: checksum(&archive)?,
            source_identity: package.identity,
            source_version: package.version.to_string(),
            native_version: env!("CARGO_PKG_VERSION").into(),
            native_sha256: checksum(&launcher)?,
        };
        let manifest_path = staging.join(MANIFEST_PATH);
        fs::write(&manifest_path, format!("{}\n", manifest.to_edn())).map_err(io_error)?;
        verify(&staging, &launcher)?;
        Ok(manifest)
    })();
    match build {
        Ok(manifest) => {
            publish_output(&staging, output, replace)?;
            verify(output, &launcher_path(output, &manifest.launcher))?;
            Ok(manifest)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
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
    verify_distribution_contents(root, &manifest)?;
    if manifest.native_version != env!("CARGO_PKG_VERSION") {
        return Err(format!(
            "distribution native version mismatch: manifest {}, launcher {}",
            manifest.native_version,
            env!("CARGO_PKG_VERSION")
        ));
    }
    Ok(manifest)
}

fn verify_distribution_contents(root: &Path, manifest: &Manifest) -> Result<(), String> {
    let launcher = launcher_path(root, &manifest.launcher);
    check_checksum(&launcher, &manifest.native_sha256, "native launcher")?;
    let archive = root.join(&manifest.archive);
    check_checksum(&archive, &manifest.archive_sha256, "package archive")?;
    let package = PackageManifest::read_archive(&archive).map_err(|error| error.to_string())?;
    if package.identity != manifest.source_identity
        || package.version.to_string() != manifest.source_version
    {
        return Err(format!(
            "distribution package mismatch: manifest {} {}, archive {} {}",
            manifest.source_identity, manifest.source_version, package.identity, package.version
        ));
    }
    Ok(())
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

fn validate_output(output: &Path, replace: bool) -> Result<(), String> {
    if !output.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(output).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "distribution output must be a real directory: {}",
            output.display()
        ));
    }
    let mut entries = fs::read_dir(output).map_err(io_error)?;
    if entries.next().is_none() {
        return Ok(());
    }
    if !replace {
        return Err(format!(
            "distribution output already exists and is not empty: {}",
            output.display()
        ));
    }
    let manifest = read(output).map_err(|_| {
        format!(
            "distribution replace output is not a verified Hara distribution: {}",
            output.display()
        )
    })?;
    verify_distribution_contents(output, &manifest).map_err(|_| {
        format!(
            "distribution replace output is not a verified Hara distribution: {}",
            output.display()
        )
    })
}

fn create_staging_output(output: &Path) -> Result<PathBuf, String> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = output
        .file_name()
        .ok_or_else(|| {
            format!(
                "distribution output must name a directory: {}",
                output.display()
            )
        })?
        .to_string_lossy();
    fs::create_dir_all(parent).map_err(io_error)?;
    for index in 0..1024 {
        let candidate = parent.join(format!(".{name}.staging-{}-{index}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(format!(
        "cannot allocate a distribution staging directory beside {}",
        output.display()
    ))
}

fn publish_output(staging: &Path, output: &Path, replace: bool) -> Result<(), String> {
    if output.exists() {
        validate_output(output, replace)?;
        fs::remove_dir_all(output).map_err(io_error)?;
    }
    fs::rename(staging, output).map_err(io_error)
}

fn has_sealed_footer(path: &Path) -> Result<bool, String> {
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "cannot inspect sealed executable {}: {error}",
            path.display()
        )
    })?;
    if metadata.len() < SEALED_FOOTER_BYTES as u64 {
        return Ok(false);
    }
    let mut file = File::open(path)
        .map_err(|error| format!("cannot read sealed executable {}: {error}", path.display()))?;
    file.seek(SeekFrom::End(-(SEALED_FOOTER_BYTES as i64)))
        .map_err(|error| format!("cannot seek sealed executable {}: {error}", path.display()))?;
    let mut footer = [0_u8; SEALED_FOOTER_BYTES];
    file.read_exact(&mut footer).map_err(|error| {
        format!(
            "cannot read sealed executable footer {}: {error}",
            path.display()
        )
    })?;
    Ok(footer[..SEALED_MAGIC.len()] == SEALED_MAGIC[..])
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
    use super::{
        build, build_replace, has_sealed_footer, inspect_sealed, install_sealed_at, read, seal,
        verify, verify_sealed, SealArchive, SealSpec, ARCHIVE_PATH, MANIFEST_PATH,
    };
    use crate::package::{self, ArtifactFile, ArtifactSpec};
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

    fn archive_fixture(root: &std::path::Path) -> PathBuf {
        let archive = root.join("source.harp");
        package::build_artifact(
            ArtifactSpec {
                identity: "hara:demo/app".into(),
                version: "1.2.3".into(),
                name: None,
                files: vec![
                    ArtifactFile {
                        path: "project.edn".into(),
                        bytes: fs::read(root.join("project.edn")).unwrap(),
                    },
                    ArtifactFile {
                        path: "src/demo/cli.hal".into(),
                        bytes: fs::read(root.join("src/demo/cli.hal")).unwrap(),
                    },
                ],
                resources: [("demo.cli".into(), "src/demo/cli.hal".into())]
                    .into_iter()
                    .collect(),
                bytecode: None,
                extensions: "{}".into(),
            },
            &archive,
        )
        .unwrap();
        archive
    }

    #[test]
    fn builds_and_verifies_a_relocatable_source_distribution() {
        let root = temp("build");
        let output = root.join("output");
        let native = root.join("native-host");
        fixture(&root);
        fs::write(&native, "native-host").unwrap();
        let archive = archive_fixture(&root);

        let manifest = build(&archive, &native, &output, "hara", "demo.cli/main").unwrap();
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

    #[test]
    fn replaces_only_an_existing_verified_distribution() {
        let root = temp("replace");
        let output = root.join("output");
        let unrelated = root.join("unrelated");
        let native = root.join("native-host");
        fixture(&root);
        fs::write(&native, "native-host").unwrap();
        let archive = archive_fixture(&root);

        build(&archive, &native, &output, "hara", "demo.cli/main").unwrap();
        let rebuilt = build_replace(&archive, &native, &output, "hara", "demo.cli/main").unwrap();
        assert_eq!(rebuilt.entry, "demo.cli/main");

        fs::create_dir_all(&unrelated).unwrap();
        fs::write(unrelated.join("notes.txt"), "do not remove").unwrap();
        assert!(
            build_replace(&archive, &native, &unrelated, "hara", "demo.cli/main")
                .unwrap_err()
                .contains("not a verified Hara distribution")
        );
        assert_eq!(
            fs::read_to_string(unrelated.join("notes.txt")).unwrap(),
            "do not remove"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn seals_a_native_host_with_a_verified_harp_payload() {
        let root = temp("sealed");
        let native = root.join("native-host");
        let output = root.join("demo");
        fixture(&root);
        fs::write(&native, "native-host").unwrap();
        let archive = archive_fixture(&root);

        let sealed = seal(&SealSpec {
            host: native.clone(),
            output: output.clone(),
            entry: "demo.cli/main".into(),
            archives: vec![SealArchive {
                path: archive,
                primary: true,
            }],
        })
        .unwrap();
        assert_eq!(sealed.entry, "demo.cli/main");
        assert_eq!(sealed.archives.len(), 1);
        assert!(sealed.archives[0].primary);
        assert!(!has_sealed_footer(&native).unwrap());
        assert_eq!(inspect_sealed(&native).unwrap(), None);
        assert_eq!(inspect_sealed(&output).unwrap(), Some(sealed.clone()));
        assert_eq!(verify_sealed(&output).unwrap(), Some(sealed));
        let installation = install_sealed_at(&output, &root.join("store"))
            .unwrap()
            .unwrap();
        assert!(installation.primary.starts_with(root.join("store")));

        let mut tampered = fs::read(&output).unwrap();
        tampered[0] ^= 1;
        fs::write(&output, tampered).unwrap();
        assert!(inspect_sealed(&output)
            .unwrap_err()
            .contains("host digest mismatch"));
        fs::remove_dir_all(root).unwrap();
    }
}
