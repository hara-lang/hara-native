//! Deterministic local package operations for the `hara package` command.
//!
//! Network reconciliation deliberately does not live here yet: package roots
//! are only activated after a registry and identity client has verified them.

use crate::kernel::{parse, Form};
use crate::project::{self, Project};
use crate::tap::{self, Tap};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub use crate::package_catalog::{catalog_from_lock, LockedPackage};

mod artifact;
mod install;
pub use artifact::{ArtifactBytecode, ArtifactFile, ArtifactSpec};
use install::{install_archive, install_archive_at, json_string, validate_recipe};

const MAX_PUBLICATION_DIAGNOSTIC_BYTES: usize = 4096;

/// A caller-supplied module for the generic precompile boundary. The caller
/// resolves paths, namespaces, and dependency context before crossing into
/// Native; Native receives only the module identity and source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompileModule {
    pub namespace: String,
    pub source: String,
}

/// Precompiles a supplied module plan into the portable bytecode container.
/// No project tree, source filename, or package profile is inspected here.
pub fn precompile(
    context: &[PrecompileModule],
    modules: &[PrecompileModule],
) -> Result<Vec<u8>, String> {
    #[cfg(not(feature = "bytecode-vm"))]
    {
        let _ = (context, modules);
        return Err("package/precompile is unavailable without the bytecode-vm feature".into());
    }
    #[cfg(feature = "bytecode-vm")]
    {
        let context = context
            .iter()
            .map(|module| crate::vm::ModuleSource {
                resource: module.namespace.as_str(),
                source: module.source.as_str(),
            })
            .collect::<Vec<_>>();
        let modules = modules
            .iter()
            .map(|module| crate::vm::ModuleSource {
                resource: module.namespace.as_str(),
                source: module.source.as_str(),
            })
            .collect::<Vec<_>>();
        crate::vm::compile_package_bytecode_bundle(&context, &modules)
    }
}

/// Builds an archive from already supplied bytes and metadata.
pub fn build_artifact(spec: ArtifactSpec, output: &Path) -> Result<PathBuf, String> {
    artifact::build(spec, output)
}

/// Package compilation walks the complete source graph and needs more stack
/// than Rust's default worker thread. Keep native API and CLI package builds
/// on the same bounded budget as the runtime broker.
pub const BUILD_THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
    64 * 1024 * 1024
} else {
    8 * 1024 * 1024
};

pub fn inspect_path(archive: &Path) -> Result<String, String> {
    artifact::inspect(archive)
}

pub(super) fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

pub(super) fn zip_error(error: zip::result::ZipError) -> String {
    error.to_string()
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn install_path(input: &Path) -> Result<PathBuf, String> {
    install_path_at(input, &install_root())
}

/// Returns the configured base directory for installed Hara packages.
///
/// Sealed executables derive a payload-specific child from this root so a
/// rebuilt executable cannot conflict with an earlier package registration at
/// the same semantic version.
pub fn install_root() -> PathBuf {
    install::dist_root()
}

/// Installs a package into an explicit distribution root. Embedders and
/// tests use this form to keep package state isolated from the user's global
/// Hara distribution.
pub fn install_path_at(input: &Path, distribution_root: &Path) -> Result<PathBuf, String> {
    if input.is_dir() {
        return Err(
            "package/install expects a supplied .harp archive; source project planning belongs to Hara"
                .into(),
        );
    }
    install_archive_at(input, distribution_root)
}

/// Handles the public `hara package` command group.
pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("check") | Some("build") => Err(
            "source package planning is owned by Hara; use `hara deploy build` or std.package.build"
                .into(),
        ),
        Some("inspect") => {
            let archive = args
                .get(1)
                .ok_or_else(|| "hara package inspect requires ARCHIVE.harp".to_owned())?;
            println!("{}", inspect_path(Path::new(archive))?);
            Ok(())
        }
        Some("profile") => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("config/packages.edn"));
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let definitions = crate::package_catalog::definitions_from_packages_edn(&source)?;
            for definition in definitions {
                let dependencies = if definition.dependencies.is_empty() {
                    String::new()
                } else {
                    format!(" depends on {}", definition.dependencies.join(", "))
                };
                println!("{}{}", definition.name, dependencies);
            }
            Ok(())
        }
        Some("install") => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let installed = install_archive(&input)?;
            println!("package install: {}", installed.display());
            Ok(())
        }
        Some("publish") => Err(github_workflow_required()),
        Some("tap") => tap_command(&args[1..]),
        Some("registry") => registry_command(&args[1..]),
        Some("sync") | Some("add") | Some("remove") | Some("update") | Some("search")
        | Some("info") => Err(format!(
            "hara package {} requires a configured GitHub registry and identity client; local package commands available now: check, build, inspect",
            args[0]
        )),
        Some("--help") | Some("-h") | None => {
            println!(
                "hara package <inspect|profile|sync|add|remove|update|publish|tap|search|info>\n\n\
                 inspect ARCHIVE.harp         print package.edn\n\
                 profile [PATH]               validate and list semantic packages\n\
                 install ARCHIVE.harp          install into HARA_DIST_HOME or ~/.hara/dist\n\
                 tap bootstrap official       install the official profile\n\
                 tap init NAME --registry PATH --identity PATH --identity-root-key ED25519_HEX\n\
                 tap add NAME --registry URL --identity URL --identity-key SHA256\n\
                 tap mirror add NAME [--registry URL] [--identity URL]\n\
                 tap list|remove NAME|verify NAME\n\
                 publication is requested by the source repository's protected GitHub workflow"
            );
            Ok(())
        }
        Some(command) => Err(format!("unknown package command: {command}")),
    }
}

pub fn github_workflow_required() -> String {
    "package/publication-github-workflow-required: push a signed source tag; the repository workflow must create the reviewed hara-packages receipt".into()
}

fn read_project(path: &Path) -> Result<Project, String> {
    project::read(path)
}

fn registry_command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("verify-request") => {
            let request = PathBuf::from(required_option(args, "--request")?);
            let identity = PathBuf::from(required_option(args, "--identity")?);
            verify_registry_request_paths(&request, &identity)?;
            println!("registry request verified: {}", request.display());
            Ok(())
        }
        _ => {
            Err("usage: hara package registry verify-request --request PATH --identity PATH".into())
        }
    }
}

pub fn verify_registry_request_paths(request: &Path, identity: &Path) -> Result<(), String> {
    let policy = fs::read_to_string(identity)
        .map_err(|error| format!("cannot read {}: {error}", identity.display()))?;
    let Form::Map(policy) = parse(&policy)? else {
        return Err("identity policy must be an EDN map".into());
    };
    let trust = policy
        .iter()
        .find(|(key, _)| matches!(key, Form::Keyword(name) if name == "identity/trust"))
        .map(|(_, value)| value);
    if !matches!(trust, Some(Form::Keyword(mode)) if mode == "github-governed") {
        return Err("registry bootstrap verifier requires :identity/trust :github-governed".into());
    }
    let intent_path = fs::read_dir(request)
        .map_err(|error| format!("cannot read {}: {error}", request.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".publisher-intent.edn"))
        })
        .ok_or("request is missing publisher intent")?;
    let intent = fs::read_to_string(&intent_path).map_err(io_error)?;
    let Form::Map(entries) = parse(&intent)? else {
        return Err("publisher intent must be an EDN map".into());
    };
    for key in [
        "intent/format",
        "tap",
        "coordinate",
        "version",
        "repository",
        "tag",
        "commit",
        "archive-sha256",
        "identity-revision",
    ] {
        if !entries
            .iter()
            .any(|(candidate, _)| matches!(candidate, Form::Keyword(name) if name == key))
        {
            return Err(format!("publisher intent is missing :{key}"));
        }
    }
    Ok(())
}

pub fn tap_command(args: &[String]) -> Result<(), String> {
    let root = tap::config_root();
    match args.first().map(String::as_str) {
        Some("add") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap add requires NAME".to_owned())?;
            let registry = option_values(args, "--registry");
            let identity = option_values(args, "--identity");
            let identity_key = option_value(args, "--identity-key")?;
            tap::add(
                &root,
                Tap {
                    name: name.clone(),
                    registry,
                    identity,
                    identity_key,
                    trust: tap::TrustMode::SignedRoot,
                },
            )?;
            println!("trusted tap {name}");
            Ok(())
        }
        Some("bootstrap") => {
            let profile = args
                .get(1)
                .ok_or_else(|| "tap bootstrap requires PROFILE".to_owned())?;
            let tap = tap::bootstrap(&root, profile)?;
            println!("bootstrapped tap {} (GitHub-governed)", tap.name);
            Ok(())
        }
        Some("mirror") if args.get(1).map(String::as_str) == Some("add") => {
            let name = args
                .get(2)
                .ok_or_else(|| "tap mirror add requires NAME".to_owned())?;
            let tap = tap::add_mirror(
                &root,
                name,
                optional_option(args, "--registry"),
                optional_option(args, "--identity"),
            )?;
            println!(
                "updated tap {} registry={} identity={}",
                tap.name,
                tap.registry.join(","),
                tap.identity.join(",")
            );
            Ok(())
        }
        Some("init") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap init requires NAME".to_owned())?;
            let registry = PathBuf::from(required_option(args, "--registry")?);
            let identity = PathBuf::from(required_option(args, "--identity")?);
            let root_key = required_option(args, "--identity-root-key")?;
            let initialized = tap::initialize(name, &registry, &identity, &root_key)?;
            tap::add(&root, initialized.tap)?;
            println!("initialized tap {name}");
            println!("identity-root fingerprint: {}", initialized.fingerprint);
            println!("scaffolded registry: {}", registry.display());
            println!("scaffolded identity: {}", identity.display());
            Ok(())
        }
        Some("remove") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap remove requires NAME".to_owned())?;
            tap::remove(&root, name)?;
            println!("removed tap {name}");
            Ok(())
        }
        Some("list") => {
            for tap in tap::load(&root)?.values() {
                println!(
                    "{} registry={} identity={}",
                    tap.name,
                    tap.registry.join(","),
                    tap.identity.join(",")
                );
            }
            Ok(())
        }
        Some("verify") => {
            let name = args
                .get(1)
                .ok_or_else(|| "tap verify requires NAME".to_owned())?;
            let tap = tap::trusted(&root, name)?;
            let scratch = scratch("verify")?;
            let result = tap::fetch_verified_policy(&tap, &scratch);
            let _ = fs::remove_dir_all(&scratch);
            let policy = result?;
            println!("tap verify: {} identity={}", tap.name, policy.revision);
            Ok(())
        }
        _ => {
            Err("usage: hara package tap <bootstrap|init|add|mirror add|remove|list|verify>".into())
        }
    }
}

pub fn publish_path(path: &Path, tap_name: &str, dry_run: bool) -> Result<String, String> {
    publish_path_with_signer(path, tap_name, dry_run, false, tap::sign)
}

/// Publish a package through a caller-owned detached-intent signer. Embedders
/// use this form when the signer is part of the host executable rather than a
/// child process named by `HARA_SIGNER`.
pub fn publish_path_with_signer<F>(
    path: &Path,
    tap_name: &str,
    dry_run: bool,
    skip_signed_tag: bool,
    signer: F,
) -> Result<String, String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    publish_path_with_signer_and_identity(path, tap_name, dry_run, skip_signed_tag, signer, None)
}

/// Integrated hosts pass their public publisher key so a missing policy grant
/// can enter the browser-backed enrollment flow and successful submissions can
/// carry an identity authorization.  The legacy external signer path remains
/// available for embedding hosts that have not adopted that flow yet.
pub fn publish_path_with_signer_and_identity<F>(
    path: &Path,
    tap_name: &str,
    dry_run: bool,
    skip_signed_tag: bool,
    signer: F,
    publisher_public_key: Option<&str>,
) -> Result<String, String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    let tap_name = if tap_name == "official" {
        "hara"
    } else {
        tap_name
    };
    let project = read_project(path)?;
    let coordinate = project::normalize_coordinate(&project.id)?;
    let (coordinate_tap, _) = split_coordinate(&coordinate)?;
    if coordinate_tap != tap_name {
        return Err(format!(
            "project id {} belongs to tap {coordinate_tap}, not {tap_name}",
            project.id
        ));
    }
    let trusted_tap = tap::trusted_or_builtin(&tap::config_root(), &tap_name)?;
    let scratch = scratch("publish")?;
    let result = publish_inner(
        &project,
        &trusted_tap,
        dry_run,
        skip_signed_tag,
        &scratch,
        &signer,
        publisher_public_key,
    );
    let _ = fs::remove_dir_all(&scratch);
    result
}

fn publish_inner<F>(
    project: &Project,
    trusted_tap: &Tap,
    dry_run: bool,
    skip_signed_tag: bool,
    scratch_root: &Path,
    signer: &F,
    publisher_public_key: Option<&str>,
) -> Result<String, String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    let policy = tap::fetch_verified_policy(trusted_tap, scratch_root)?;
    let tag = project.release_tag.clone();
    let (source_reference, commit) = source_release(&project.root, &tag, skip_signed_tag)?;
    let repository = tap::git(&project.root, ["config", "--get", "remote.origin.url"])?;
    let recipe = validate_recipe(project)?;
    let project_sha256 = file_sha256(&project.root.join("project.edn"))?;
    let recipe_sha256 = file_sha256(&recipe)?;
    let coordinate = project::normalize_coordinate(&project.id)?;
    let intent = tap::canonical_recipe_intent(
        &coordinate,
        &project.version.to_string(),
        &repository,
        &source_reference,
        &commit,
        &project_sha256,
        &recipe_sha256,
        &trusted_tap.name,
        &policy.revision,
    );
    let (key_id, signature) = signer(intent.as_bytes())?;
    if let Err(error) = tap::authorize(&policy, &key_id, &coordinate, intent.as_bytes(), &signature)
    {
        if !dry_run {
            if let Some(public_key) = publisher_public_key {
                crate::identity_tool::request_publisher_grant_with_signer(
                    &coordinate,
                    &intent,
                    &policy.revision,
                    public_key,
                    signer,
                )?;
            }
        }
        return Err(error);
    }
    if dry_run {
        let status = if skip_signed_tag {
            "untagged-source publish preflight (remote default head verified)"
        } else {
            "publish recipe verified"
        };
        return Ok(format!(
            "{status}: {} {} tap={} recipe=sha256:{}",
            coordinate, project.version, trusted_tap.name, recipe_sha256,
        ));
    }
    let endpoint = trusted_tap
        .registry
        .first()
        .ok_or("official tap has no publication endpoint")?;
    let authorization = match publisher_public_key {
        Some(public_key) => crate::identity_tool::request_publication_authorization_with_signer(
            &coordinate,
            &intent,
            &policy.revision,
            public_key,
            signer,
        )?,
        None => "null".into(),
    };
    let body = format!(
        "{{\"intent\":{},\"key_id\":\"{}\",\"signature\":\"{}\",\"authorization\":{}}}",
        json_string(&intent),
        key_id,
        signature,
        authorization,
    );
    let output = std::process::Command::new("curl")
        .args([
            "--fail-with-body",
            "--silent",
            "--show-error",
            "-H",
            "content-type: application/json",
            "--data-binary",
            &body,
            &format!("{}/v1/publications", endpoint.trim_end_matches('/')),
        ])
        .output()
        .map_err(|error| format!("cannot start publication client: {error}"))?;
    if !output.status.success() {
        return Err(publication_request_error(&output.stderr, &output.stdout));
    }
    Ok(format!(
        "publish requested: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn publication_request_error(stderr: &[u8], response: &[u8]) -> String {
    let transport = publication_diagnostic(stderr);
    let body = publication_diagnostic(response);
    match (transport.is_empty(), body.is_empty()) {
        (true, true) => "publication request failed".into(),
        (false, true) => format!("publication request failed: {transport}"),
        (true, false) => format!("publication request failed: {body}"),
        (false, false) => format!("publication request failed: {transport}: {body}"),
    }
}

fn publication_diagnostic(bytes: &[u8]) -> String {
    let length = bytes.len().min(MAX_PUBLICATION_DIAGNOSTIC_BYTES);
    let text = String::from_utf8_lossy(&bytes[..length]).trim().to_owned();
    if bytes.len() > length && !text.is_empty() {
        format!("{text}…")
    } else {
        text
    }
}

/// Resolves the commit that a publication intent names. Untagged publication
/// requires a clean checkout whose HEAD is exactly the origin default branch.
pub fn source_release_commit(
    root: &Path,
    tag: &str,
    skip_signed_tag: bool,
) -> Result<String, String> {
    source_release(root, tag, skip_signed_tag).map(|(_, commit)| commit)
}

fn source_release(
    root: &Path,
    tag: &str,
    skip_signed_tag: bool,
) -> Result<(String, String), String> {
    if !skip_signed_tag {
        tap::git(root, ["tag", "-v", tag])
            .map_err(|error| format!("publish requires a valid signed tag {tag}: {error}"))?;
        return Ok((tag.into(), tap::git(root, ["rev-list", "-n", "1", tag])?));
    }

    let status = tap::git(root, ["status", "--porcelain", "--untracked-files=all"])?;
    if !status.is_empty() {
        return Err("publish without a signed tag requires a clean worktree".into());
    }
    let commit = tap::git(root, ["rev-parse", "HEAD"])?;
    let remote_head =
        tap::git(root, ["ls-remote", "--symref", "origin", "HEAD"]).map_err(|error| {
            format!("publish without a signed tag cannot resolve origin default branch: {error}")
        })?;
    let branch = remote_head
        .lines()
        .find_map(|line| {
            let (reference, name) = line.split_once('\t')?;
            if name == "HEAD" {
                reference.strip_prefix("ref: refs/heads/")
            } else {
                None
            }
        })
        .ok_or("publish without a signed tag cannot determine origin default branch")?;
    let remote_ref = format!("refs/heads/{branch}");
    let remote_commit = tap::git(root, ["ls-remote", "origin", remote_ref.as_str()])
        .map_err(|error| {
            format!(
                "publish without a signed tag cannot read origin default branch {branch}: {error}"
            )
        })?
        .split_whitespace()
        .next()
        .ok_or_else(|| {
            format!("publish without a signed tag cannot read origin default branch {branch}")
        })?
        .to_owned();
    if remote_commit != commit {
        return Err(format!(
            "publish without a signed tag requires HEAD {commit} to match origin remote default branch {branch} at {remote_commit}"
        ));
    }
    Ok((format!("untagged:{branch}"), commit))
}

fn option_value(args: &[String], flag: &str) -> Result<String, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("publish requires {flag}"))?;
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}
fn required_option(args: &[String], flag: &str) -> Result<String, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("tap init requires {flag}"))?;
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}
fn option_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, value)| value.as_str() == flag)
        .filter_map(|(index, _)| args.get(index + 1).cloned())
        .collect()
}
fn optional_option(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}
fn split_coordinate(value: &str) -> Result<(&str, &str), String> {
    let (tap, package) = value
        .split_once(':')
        .ok_or_else(|| format!("package coordinate must use TAP:owner/name: {value}"))?;
    if tap.is_empty() || package.is_empty() || package.contains(':') {
        return Err(format!("invalid tap-qualified package coordinate: {value}"));
    }
    Ok((tap, package))
}
fn scratch(label: &str) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join(format!("hara-{label}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(io_error)?;
    }
    fs::create_dir_all(&root).map_err(io_error)?;
    Ok(root)
}
fn file_sha256(path: &Path) -> Result<String, String> {
    Ok(hex(&Sha256::digest(fs::read(path).map_err(io_error)?)))
}

#[cfg(test)]
mod tests;
