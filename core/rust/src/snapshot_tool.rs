//! Native CLI adapter for portable HSS0 snapshots.

use crate::core;
use crate::kernel::{self, Form};
use crate::snapshot::{
    self, Digest, LibraryRef, NamespaceImage, ResolvedSnapshot, SecretRequirement,
    SnapshotArtifact, SnapshotManifest,
};
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("build") => build(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        Some("diff") => diff(&args[1..]),
        Some(command) => Err(format!("unknown snapshot command: {command}")),
        None => Err("usage: hara snapshot <build|verify|inspect|diff> ...".into()),
    }
}

fn build(args: &[String]) -> Result<(), String> {
    let source = args
        .first()
        .ok_or("usage: hara snapshot build SNAPSHOT.edn --output FILE.hss")?;
    let output = option(args, "--output").ok_or("snapshot build requires --output FILE.hss")?;
    println!("{}", build_paths(Path::new(source), Path::new(output))?);
    Ok(())
}

pub fn build_paths(source: &Path, output: &Path) -> Result<String, String> {
    let source_path = source.to_path_buf();
    let source_text = fs::read_to_string(&source_path)
        .map_err(|error| format!("cannot read {}: {error}", source_path.display()))?;
    let form = kernel::parse(&source_text)
        .map_err(|error| format!("cannot parse {}: {error}", source_path.display()))?;
    let artifact = artifact_from_form(&form, source_path.parent().unwrap_or(Path::new(".")))?;
    let bytes = snapshot::encode(&artifact)?;
    fs::write(output, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    let resolved = resolve_with_declared_base(
        &artifact,
        &form,
        source_path.parent().unwrap_or(Path::new(".")),
    )?;
    Ok(format!(
        "snapshot build: {} {} bytes{}",
        snapshot::hex(&resolved.digest),
        bytes.len(),
        if artifact.is_incremental() {
            " incremental"
        } else {
            ""
        }
    ))
}

fn verify(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("usage: hara snapshot verify FILE.hss [--base BASE.hss]")?;
    println!(
        "{}",
        verify_paths(Path::new(path), option(args, "--base").map(Path::new))?
    );
    Ok(())
}

pub fn verify_paths(path: &Path, base: Option<&Path>) -> Result<String, String> {
    let artifact = read_artifact(path)?;
    let base = base.map(read_full_resolved).transpose()?;
    let resolved = artifact.resolve(base.as_ref())?;
    Ok(format!(
        "snapshot verify: {} namespaces={} libraries={} secrets={}",
        snapshot::hex(&resolved.digest),
        resolved.manifest.namespaces.len(),
        resolved.manifest.libraries.len(),
        resolved.manifest.secrets.len()
    ))
}

fn inspect(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or("usage: hara snapshot inspect FILE.hss")?;
    print!("{}", inspect_path(Path::new(path))?);
    Ok(())
}

pub fn inspect_path(path: &Path) -> Result<String, String> {
    use std::fmt::Write as _;
    let artifact = read_artifact(path)?;
    let mut output = String::new();
    writeln!(output, "format: HSS0").unwrap();
    writeln!(
        output,
        "artifact: {}",
        snapshot::hex(&snapshot::artifact_digest(&fs::read(path).map_err(io)?))
    )
    .unwrap();
    writeln!(
        output,
        "base: {}",
        artifact
            .base
            .as_ref()
            .map(snapshot::hex)
            .unwrap_or_else(|| "none".into())
    )
    .unwrap();
    writeln!(output, "language: {}", artifact.manifest.language_version).unwrap();
    writeln!(output, "libraries: {}", artifact.manifest.libraries.len()).unwrap();
    for library in &artifact.manifest.libraries {
        writeln!(
            output,
            "  {} {} {}",
            library.id,
            library.version,
            snapshot::hex(&library.digest)
        )
        .unwrap();
    }
    writeln!(output, "namespaces: {}", artifact.manifest.namespaces.len()).unwrap();
    for namespace in &artifact.manifest.namespaces {
        writeln!(
            output,
            "  {} {} {}",
            namespace.name,
            snapshot::hex(&namespace.digest),
            if namespace.halc.is_some() {
                "embedded"
            } else {
                "inherited"
            }
        )
        .unwrap();
    }
    writeln!(output, "entrypoints:").unwrap();
    for (name, target) in &artifact.manifest.entrypoints {
        writeln!(output, "  {name} -> {target}").unwrap();
    }
    writeln!(
        output,
        "secret requirements: {}",
        artifact.manifest.secrets.len()
    )
    .unwrap();
    for secret in &artifact.manifest.secrets {
        writeln!(
            output,
            "  {} required={} version={} purpose={}",
            secret.id,
            secret.required,
            secret.version.as_deref().unwrap_or("unspecified"),
            secret.purpose
        )
        .unwrap();
    }
    Ok(output)
}

fn diff(args: &[String]) -> Result<(), String> {
    let [left, right] = args else {
        return Err("usage: hara snapshot diff LEFT.hss RIGHT.hss".into());
    };
    print!("{}", diff_paths(Path::new(left), Path::new(right))?);
    Ok(())
}

pub fn diff_paths(left: &Path, right: &Path) -> Result<String, String> {
    use std::fmt::Write as _;
    let left = read_full_resolved(left)?;
    let right_artifact = read_artifact(right)?;
    let right = if right_artifact.base == Some(left.digest) {
        right_artifact.resolve(Some(&left))?
    } else {
        right_artifact.resolve(None)?
    };
    let mut output = String::new();
    writeln!(output, "left:  {}", snapshot::hex(&left.digest)).unwrap();
    writeln!(output, "right: {}", snapshot::hex(&right.digest)).unwrap();
    output.push_str(&report_set_diff(
        "namespaces",
        left.manifest
            .namespaces
            .iter()
            .map(|value| value.name.as_str()),
        right
            .manifest
            .namespaces
            .iter()
            .map(|value| value.name.as_str()),
    ));
    output.push_str(&report_set_diff(
        "entrypoints",
        left.manifest.entrypoints.keys().map(String::as_str),
        right.manifest.entrypoints.keys().map(String::as_str),
    ));
    output.push_str(&report_set_diff(
        "state",
        left.manifest.initial_state.keys().map(String::as_str),
        right.manifest.initial_state.keys().map(String::as_str),
    ));
    output.push_str(&report_set_diff(
        "secrets",
        left.manifest.secrets.iter().map(|value| value.id.as_str()),
        right.manifest.secrets.iter().map(|value| value.id.as_str()),
    ));
    Ok(output)
}

fn artifact_from_form(form: &Form, root: &Path) -> Result<SnapshotArtifact, String> {
    let entries = as_map(form, "snapshot document must be a map")?;
    reject_secret_values(entries)?;
    let language_version = string(
        required(entries, "snapshot/language-version")?,
        ":snapshot/language-version",
    )?;
    let dependency_lock_digest =
        digest_form(required(entries, "snapshot/dependency-lock-digest")?)?;
    let base_path = optional_string(entries, "snapshot/base")?;
    let base = base_path
        .as_deref()
        .map(|path| read_full_resolved(&root.join(path)))
        .transpose()?;

    let libraries = optional_vector(entries, "snapshot/libraries")?
        .unwrap_or_default()
        .iter()
        .map(|form| {
            let library = as_map(form, "snapshot library must be a map")?;
            Ok(LibraryRef {
                id: name(required(library, "library/id")?, ":library/id")?,
                version: string(required(library, "library/version")?, ":library/version")?,
                digest: digest_form(required(library, "library/digest")?)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let namespaces = optional_vector(entries, "snapshot/namespaces")?
        .unwrap_or_default()
        .iter()
        .map(|form| namespace(form, root, base.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;

    let entrypoints = optional_map(entries, "snapshot/entrypoints")?
        .unwrap_or_default()
        .iter()
        .map(|(key, value)| {
            Ok((
                name(key, "entrypoint name")?,
                name(value, "entrypoint target")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let initial_state = optional_map(entries, "snapshot/initial-state")?
        .unwrap_or_default()
        .iter()
        .map(|(key, value)| Ok((name(key, "state name")?, core::form_to_value(value)?)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let capabilities = optional_collection(entries, "snapshot/capabilities")?
        .unwrap_or_default()
        .iter()
        .map(|value| name(value, "capability"))
        .collect::<Result<BTreeSet<_>, _>>()?;

    let secrets = optional_vector(entries, "snapshot/secrets")?
        .unwrap_or_default()
        .iter()
        .map(secret_requirement)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(SnapshotArtifact {
        base: base.as_ref().map(|value| value.digest),
        manifest: SnapshotManifest {
            language_version,
            dependency_lock_digest,
            libraries,
            namespaces,
            entrypoints,
            initial_state,
            capabilities,
            secrets,
            accelerators: vec![],
        },
    })
}

fn resolve_with_declared_base(
    artifact: &SnapshotArtifact,
    form: &Form,
    root: &Path,
) -> Result<ResolvedSnapshot, String> {
    let entries = as_map(form, "snapshot document must be a map")?;
    let base = optional_string(entries, "snapshot/base")?
        .map(|path| read_full_resolved(&root.join(path)))
        .transpose()?;
    artifact.resolve(base.as_ref())
}

fn namespace(
    form: &Form,
    root: &Path,
    base: Option<&ResolvedSnapshot>,
) -> Result<NamespaceImage, String> {
    let entries = as_map(form, "snapshot namespace must be a map")?;
    let name = name(required(entries, "namespace/name")?, ":namespace/name")?;
    if optional_bool(entries, "namespace/inherit")?.unwrap_or(false) {
        let inherited = base
            .and_then(|base| {
                base.manifest
                    .namespaces
                    .iter()
                    .find(|value| value.name == name)
            })
            .ok_or_else(|| format!("snapshot inherited namespace is absent from base: {name}"))?;
        return Ok(NamespaceImage {
            name,
            digest: inherited.digest,
            halc: None,
        });
    }
    let path = optional_string(entries, "namespace/halc")?.ok_or_else(|| {
        format!("snapshot namespace {name} requires :namespace/halc or :namespace/inherit")
    })?;
    let bytes =
        fs::read(root.join(&path)).map_err(|error| format!("cannot read {path}: {error}"))?;
    let module = kernel::halc::decode_halc(&bytes)
        .map_err(|error| format!("invalid HALC namespace {name}: {error}"))?;
    if module.namespace != name {
        return Err(format!(
            "snapshot namespace name mismatch: declared {name}, HALC {}",
            module.namespace
        ));
    }
    Ok(NamespaceImage {
        name,
        digest: Sha256::digest(&bytes).into(),
        halc: Some(bytes),
    })
}

fn secret_requirement(form: &Form) -> Result<SecretRequirement, String> {
    let entries = as_map(form, "secret requirement must be a map")?;
    reject_secret_values(entries)?;
    Ok(SecretRequirement {
        id: name(required(entries, "secret/id")?, ":secret/id")?,
        purpose: string(required(entries, "secret/purpose")?, ":secret/purpose")?,
        required: optional_bool(entries, "secret/required")?.unwrap_or(true),
        version: optional_string(entries, "secret/provider-version")?,
    })
}

fn reject_secret_values(entries: &[(Form, Form)]) -> Result<(), String> {
    for (key, _) in entries {
        if matches!(key, Form::Keyword(value) if matches!(value.as_str(), "secret/value" | "secret/bytes" | "secret/key"))
        {
            return Err(
                "snapshot secret material is forbidden; declare only a secret requirement".into(),
            );
        }
    }
    Ok(())
}

fn read_artifact(path: &Path) -> Result<SnapshotArtifact, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    snapshot::decode(&bytes)
}

fn read_full_resolved(path: &Path) -> Result<ResolvedSnapshot, String> {
    let artifact = read_artifact(path)?;
    if artifact.is_incremental() {
        return Err(format!(
            "{} is incremental; provide a resolved full base",
            path.display()
        ));
    }
    artifact.resolve(None)
}

fn report_set_diff<'a>(
    kind: &str,
    left: impl Iterator<Item = &'a str>,
    right: impl Iterator<Item = &'a str>,
) -> String {
    use std::fmt::Write as _;
    let mut output = String::new();
    let left = left.collect::<BTreeSet<_>>();
    let right = right.collect::<BTreeSet<_>>();
    for value in right.difference(&left) {
        writeln!(output, "+ {kind} {value}").unwrap();
    }
    for value in left.difference(&right) {
        writeln!(output, "- {kind} {value}").unwrap();
    }
    output
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn as_map<'a>(form: &'a Form, message: &str) -> Result<&'a [(Form, Form)], String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err(message.into()),
    }
}

fn required<'a>(entries: &'a [(Form, Form)], key: &str) -> Result<&'a Form, String> {
    get(entries, key).ok_or_else(|| format!("missing :{key}"))
}

fn get<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}

fn optional_map<'a>(
    entries: &'a [(Form, Form)],
    key: &str,
) -> Result<Option<&'a [(Form, Form)]>, String> {
    get(entries, key)
        .map(|value| as_map(value, &format!(":{key} must be a map")))
        .transpose()
}

fn optional_vector<'a>(
    entries: &'a [(Form, Form)],
    key: &str,
) -> Result<Option<&'a [Form]>, String> {
    get(entries, key)
        .map(|value| match value {
            Form::Vector(values) => Ok(values.as_slice()),
            _ => Err(format!(":{key} must be a vector")),
        })
        .transpose()
}

fn optional_collection<'a>(
    entries: &'a [(Form, Form)],
    key: &str,
) -> Result<Option<&'a [Form]>, String> {
    get(entries, key)
        .map(|value| match value {
            Form::Vector(values) | Form::Set(values) => Ok(values.as_slice()),
            _ => Err(format!(":{key} must be a vector or set")),
        })
        .transpose()
}

fn optional_string(entries: &[(Form, Form)], key: &str) -> Result<Option<String>, String> {
    get(entries, key)
        .map(|value| string(value, &format!(":{key}")))
        .transpose()
}

fn optional_bool(entries: &[(Form, Form)], key: &str) -> Result<Option<bool>, String> {
    get(entries, key)
        .map(|value| match value {
            Form::Bool(value) => Ok(*value),
            _ => Err(format!(":{key} must be boolean")),
        })
        .transpose()
}

fn string(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string")),
    }
}

fn name(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) | Form::Symbol(value) | Form::Keyword(value) if !value.is_empty() => {
            Ok(value.clone())
        }
        _ => Err(format!("{label} must be a non-empty name")),
    }
}

fn digest_form(form: &Form) -> Result<Digest, String> {
    let value = string(form, "digest")?;
    let value = value.strip_prefix("sha256:").unwrap_or(&value);
    if value.len() != 64 {
        return Err("digest must contain 64 hexadecimal characters".into());
    }
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "digest contains non-hexadecimal characters")?;
    }
    Ok(digest)
}

fn io(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_inline_secret_material() {
        let form = kernel::parse(
            "{:snapshot/language-version \"0.1\" \
              :snapshot/dependency-lock-digest \"0000000000000000000000000000000000000000000000000000000000000000\" \
              :snapshot/secrets [{:secret/id :api :secret/purpose \"api\" :secret/value \"no\"}]}"
        ).unwrap();
        assert!(artifact_from_form(&form, Path::new("."))
            .unwrap_err()
            .contains("secret material"));
    }

    #[test]
    fn builds_a_minimal_manifest() {
        let form = kernel::parse(
            "{:snapshot/language-version \"0.1\" \
              :snapshot/dependency-lock-digest \"0000000000000000000000000000000000000000000000000000000000000000\" \
              :snapshot/entrypoints {:api app/handler} \
              :snapshot/initial-state {:flags {:enabled true}} \
              :snapshot/secrets [{:secret/id :api :secret/purpose \"sign\"}]}"
        ).unwrap();
        let artifact = artifact_from_form(&form, Path::new(".")).unwrap();
        assert_eq!(artifact.manifest.entrypoints["api"], "app/handler");
        assert_eq!(artifact.manifest.secrets[0].id, "api");
        assert!(artifact.resolve(None).is_ok());
    }
}
