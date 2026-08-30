//! Publisher identity management for the Hara CLI.
//!
//! Private keys remain behind `HARA_SIGNER`.  This client exchanges only
//! canonical enrollment bytes, detached signatures and public key material.

use crate::{
    kernel::{parse, Form},
    project, tap,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_IDENTITY_ENDPOINT: &str = "https://id.hara-lang.org";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublisherDevice {
    id: String,
    secret: String,
    verification_uri: String,
    challenge: String,
    interval: Duration,
}

pub fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h") => {
            usage();
            Ok(())
        }
        Some("login") => {
            println!("{}/github/start", endpoint().trim_end_matches('/'));
            Ok(())
        }
        Some("enroll") => {
            let public_key = env::var("HARA_SIGNER_PUBLIC_KEY")
                .map_err(|_| "id enroll requires HARA_SIGNER_PUBLIC_KEY".to_owned())?;
            enroll_with_signer(&args[1..], &public_key, tap::sign)
        }
        Some("status") => get("/v1/status", &args[1..]),
        Some("namespace") => get("/v1/namespaces", &args[1..]),
        Some("grant") => Err("publisher grants are requested automatically by `hara-native publish`; an offline policy maintainer finalizes the reviewed grant with `hara-native id policy grant`".into()),
        Some("policy") => Err("identity policy changes require `hara-native id policy grant` with an explicit offline root key file".into()),
        Some("key") => key_command(&args[1..]),
        Some(command) => Err(format!("unknown id command: {command}")),
    }
}

/// Applies one reviewed, exact-coordinate grant to a policy and signs the
/// exact replacement bytes with the caller-owned offline root signer. This
/// function never contacts GitHub and refuses to replace a different key or
/// authorization service key under an existing identifier.
pub fn grant_policy_with_signer<F>(
    args: &[String],
    root_public_key: &str,
    signer: F,
) -> Result<(), String>
where
    F: Fn(&[u8]) -> Result<String, String>,
{
    let parsed = PolicyGrantArguments::parse(args)?;
    validate_hex(root_public_key, 32, "identity root public key")?;
    validate_hex(&parsed.public_key, 32, "publisher public key")?;
    validate_hex(
        &parsed.authorization_public_key,
        32,
        "Identity publication authorization public key",
    )?;
    if !parsed
        .github_subject
        .bytes()
        .all(|byte| byte.is_ascii_digit())
    {
        return Err("--github-subject must be the stable numeric GitHub account id".into());
    }
    let coordinate = project::normalize_coordinate(&parsed.coordinate)?;
    if !coordinate.starts_with("hara:") {
        return Err("--coordinate must belong to the official hara tap".into());
    }
    let policy_path = absolute_file(&parsed.identity, "--identity")?;
    let source = fs::read_to_string(&policy_path)
        .map_err(|error| format!("cannot read {}: {error}", policy_path.display()))?;
    let updated = policy_with_grant(
        &source,
        root_public_key,
        &parsed.key_id,
        &parsed.public_key,
        &parsed.github_subject,
        &coordinate,
        &parsed.authorization_public_key,
    )?;
    let signature = signer(updated.as_bytes())?;
    verify_root_signature(root_public_key, updated.as_bytes(), &signature)?;
    let signature_path = policy_path.with_file_name("identity.edn.sig");
    if parsed.dry_run {
        print!("{updated}");
        println!("signature={signature}");
        println!(
            "would write {} and {}",
            policy_path.display(),
            signature_path.display()
        );
        return Ok(());
    }
    fs::write(&policy_path, &updated)
        .map_err(|error| format!("cannot write {}: {error}", policy_path.display()))?;
    fs::write(&signature_path, format!("{signature}\n"))
        .map_err(|error| format!("cannot write {}: {error}", signature_path.display()))?;
    println!(
        "signed publisher grant: {} -> {}",
        parsed.key_id, coordinate
    );
    println!(
        "updated {} and {}",
        policy_path.display(),
        signature_path.display()
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PolicyGrantArguments {
    identity: PathBuf,
    key_id: String,
    public_key: String,
    github_subject: String,
    coordinate: String,
    authorization_public_key: String,
    dry_run: bool,
}

impl PolicyGrantArguments {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut values = std::collections::BTreeMap::new();
        let mut dry_run = false;
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            if argument == "--dry-run" {
                dry_run = true;
                index += 1;
                continue;
            }
            if !matches!(
                argument.as_str(),
                "--identity"
                    | "--key-id"
                    | "--public-key"
                    | "--github-subject"
                    | "--coordinate"
                    | "--authorization-public-key"
            ) {
                return Err(format!("unknown id policy grant option: {argument}"));
            }
            let value = args
                .get(index + 1)
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| format!("{argument} requires a value"))?;
            if values.insert(argument.clone(), value.clone()).is_some() {
                return Err(format!("{argument} may be supplied only once"));
            }
            index += 2;
        }
        let mut required = |name: &str| {
            values
                .remove(name)
                .ok_or_else(|| format!("id policy grant requires {name}"))
        };
        let key_id = required("--key-id")?;
        validate_key_id(&key_id)?;
        Ok(Self {
            identity: PathBuf::from(required("--identity")?),
            key_id,
            public_key: required("--public-key")?,
            github_subject: required("--github-subject")?,
            coordinate: required("--coordinate")?,
            authorization_public_key: required("--authorization-public-key")?,
            dry_run,
        })
    }
}

fn policy_with_grant(
    source: &str,
    root_public_key: &str,
    key_id: &str,
    public_key: &str,
    github_subject: &str,
    coordinate: &str,
    authorization_public_key: &str,
) -> Result<String, String> {
    let Form::Map(mut policy) = parse(source)? else {
        return Err("identity policy must be an EDN map".into());
    };
    let root = policy_value(&policy, "identity/root-key")
        .and_then(form_string)
        .ok_or("identity policy is missing string :identity/root-key")?;
    if root != root_public_key {
        return Err(
            "offline root key does not match :identity/root-key; refusing to change policy".into(),
        );
    }
    let authorization =
        policy_value(&policy, "identity/publish-authorization-key").and_then(form_string);
    if let Some(existing) = authorization {
        if existing != authorization_public_key {
            return Err(
                "identity policy already names a different publication authorization key".into(),
            );
        }
    } else {
        policy.push((
            Form::Keyword("identity/publish-authorization-key".into()),
            Form::String(authorization_public_key.into()),
        ));
    }
    let keys = policy_value_mut(&mut policy, "publisher-keys")
        .ok_or("identity policy is missing :publisher-keys")?;
    let Form::Map(keys) = keys else {
        return Err("identity policy :publisher-keys must be an EDN map".into());
    };
    let wanted = Form::Map(vec![
        (
            Form::Keyword("public-key".into()),
            Form::String(public_key.into()),
        ),
        (
            Form::Keyword("github-subject".into()),
            Form::String(github_subject.into()),
        ),
        (
            Form::Keyword("coordinates".into()),
            Form::Vector(vec![Form::String(coordinate.into())]),
        ),
        (
            Form::Keyword("namespace-owners".into()),
            Form::Vector(vec![]),
        ),
        (Form::Keyword("revoked".into()), Form::Bool(false)),
    ]);
    if let Some((_, existing)) = keys
        .iter()
        .find(|(candidate, _)| form_string(candidate) == Some(key_id))
    {
        if existing != &wanted {
            return Err(format!(
                "publisher key {key_id} already has a different policy grant"
            ));
        }
    } else {
        keys.push((Form::String(key_id.into()), wanted));
    }
    Ok(format!("{}\n", Form::Map(policy)))
}

fn policy_value<'a>(entries: &'a [(Form, Form)], name: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(key, value)| {
        (matches!(key, Form::Keyword(candidate) if candidate == name)).then_some(value)
    })
}

fn policy_value_mut<'a>(entries: &'a mut [(Form, Form)], name: &str) -> Option<&'a mut Form> {
    entries.iter_mut().find_map(|(key, value)| {
        (matches!(key, Form::Keyword(candidate) if candidate == name)).then_some(value)
    })
}

fn form_string(value: &Form) -> Option<&str> {
    match value {
        Form::String(value) => Some(value),
        _ => None,
    }
}

fn absolute_file(path: &Path, option: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{option} must be an absolute path"));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(format!("{option} must name a regular policy file"));
    }
    Ok(path.to_path_buf())
}

fn validate_key_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("--key-id must use 1-128 ASCII letters, numbers, '.', '_' or '-'".into());
    }
    Ok(())
}

fn verify_root_signature(public_key: &str, message: &[u8], signature: &str) -> Result<(), String> {
    let public_key = hex_bytes(public_key, 32, "identity root public key")?;
    let signature = hex_bytes(signature, 64, "identity root signature")?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| "identity root public key has the wrong length")?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| "identity root signature has the wrong length")?;
    let key = VerifyingKey::from_bytes(&public_key)
        .map_err(|error| format!("identity root public key is invalid: {error}"))?;
    let signature = Signature::from_bytes(&signature);
    key.verify(message, &signature)
        .map_err(|_| "offline root signer did not produce a valid policy signature".into())
}

fn hex_bytes(value: &str, expected: usize, label: &str) -> Result<Vec<u8>, String> {
    validate_hex(value, expected, label)?;
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| format!("{label} must be lowercase hexadecimal"))
        })
        .collect()
}

/// Starts the browser-backed grant request used when a locally signed policy
/// does not yet authorize the publisher key.  The service receives only public
/// material and the detached proof over its fresh device challenge.
pub fn request_publisher_grant_with_signer<F>(
    coordinate: &str,
    intent: &str,
    identity_revision: &str,
    public_key: &str,
    signer: F,
) -> Result<(), String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    match complete_device_flow(
        "grant",
        coordinate,
        intent,
        identity_revision,
        public_key,
        signer,
    )? {
        JsonValue::Object(result)
            if result.get("status").and_then(JsonValue::as_str) == Some("grant-pending") =>
        {
            let review = result
                .get("reviewUrl")
                .and_then(JsonValue::as_str)
                .unwrap_or("the identity review queue");
            Err(format!("publisher grant is pending root-policy approval: {review}; rerun the same hara-native publish command after the signed policy PR merges"))
        }
        _ => Err("identity service returned an invalid publisher grant result".into()),
    }
}

/// Obtains the short-lived authorization submitted alongside a canonical
/// publisher intent.  Registry intake independently verifies both this proof
/// and the root-signed publisher grant.
pub fn request_publication_authorization_with_signer<F>(
    coordinate: &str,
    intent: &str,
    identity_revision: &str,
    public_key: &str,
    signer: F,
) -> Result<String, String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    let result = complete_device_flow(
        "authorize",
        coordinate,
        intent,
        identity_revision,
        public_key,
        signer,
    )?;
    let authorization = result
        .get("authorization")
        .ok_or("identity service did not return a publication authorization")?;
    serde_json::to_string(authorization)
        .map_err(|error| format!("cannot encode publication authorization: {error}"))
}

fn complete_device_flow<F>(
    mode: &str,
    coordinate: &str,
    intent: &str,
    identity_revision: &str,
    public_key: &str,
    signer: F,
) -> Result<JsonValue, String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    validate_hex(public_key, 32, "publisher public key")?;
    let created = request_json(
        "POST",
        "/v1/publisher/devices",
        Some(json!({ "mode": mode })),
        None,
    )?;
    let device = publisher_device(&created)?;
    let proof_bytes = publisher_proof_message(&device.id, &device.challenge, mode);
    let (key_id, proof) = signer(proof_bytes.as_bytes())?;
    let proof_response = request_json(
        "POST",
        &format!("/v1/publisher/devices/{}/proof", device.id),
        Some(json!({
            "keyId": key_id,
            "publicKey": public_key,
            "proof": proof,
            "coordinate": coordinate,
            "intent": intent,
            "identityRevision": identity_revision,
        })),
        Some(&device.secret),
    )?;
    if proof_response.get("status").and_then(JsonValue::as_str) != Some("pending-confirmation") {
        return Err("identity service did not accept the publisher key proof".into());
    }
    println!(
        "Open {} and confirm the publisher request in GitHub.",
        device.verification_uri
    );
    wait_for_device(&device)
}

pub fn publisher_proof_message(id: &str, challenge: &str, mode: &str) -> String {
    format!("hara-publisher-device/1\n{id}\n{challenge}\n{mode}\n")
}

fn publisher_device(value: &JsonValue) -> Result<PublisherDevice, String> {
    let object = value
        .as_object()
        .ok_or("identity service returned an invalid device response")?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("identity device response is missing {key}"))
    };
    let interval = object
        .get("interval")
        .and_then(JsonValue::as_u64)
        .unwrap_or(2)
        .clamp(1, 10);
    Ok(PublisherDevice {
        id: string("deviceId")?,
        secret: string("deviceSecret")?,
        verification_uri: string("verificationUri")?,
        challenge: string("challenge")?,
        interval: Duration::from_secs(interval),
    })
}

fn wait_for_device(device: &PublisherDevice) -> Result<JsonValue, String> {
    let timeout = env::var("HARA_PUBLISH_DEVICE_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (30..=900).contains(value))
        .unwrap_or(300);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        if Instant::now() >= deadline {
            return Err("publisher browser confirmation timed out; rerun hara-native publish to start a new device request".into());
        }
        thread::sleep(device.interval);
        let status = request_json(
            "GET",
            &format!("/v1/publisher/devices/{}", device.id),
            None,
            Some(&device.secret),
        )?;
        match status.get("status").and_then(JsonValue::as_str) {
            Some("pending-proof" | "pending-confirmation") => continue,
            Some("grant-pending" | "authorized") => return Ok(status),
            Some(state) => {
                return Err(format!(
                    "identity service returned unsupported publisher state: {state}"
                ))
            }
            None => return Err("identity service returned publisher status without a state".into()),
        }
    }
}

/// Enroll a publisher key with a caller-owned signer. This lets an embedding
/// host keep the private key in-process while the legacy CLI retains its
/// external `HARA_SIGNER` adapter.
pub fn enroll_with_signer<F>(args: &[String], public_key: &str, signer: F) -> Result<(), String>
where
    F: Fn(&[u8]) -> Result<(String, String), String>,
{
    let owner = required_option(args, "--owner")?;
    let tap_name = optional_option(args, "--tap").unwrap_or_else(|| "hara".into());
    let tap_name = if tap_name == "official" {
        "hara".to_owned()
    } else {
        tap_name
    };
    validate_hex(public_key, 32, "HARA_SIGNER_PUBLIC_KEY")?;
    let challenge = if let Some(challenge) = optional_option(args, "--challenge") {
        challenge
    } else {
        fetch_challenge(&owner)?
    };
    let request = canonical_enrollment(&tap_name, &owner, public_key, &challenge);
    let (key_id, signature) = signer(request.as_bytes())?;
    if args.iter().any(|arg| arg == "--dry-run") {
        print!("{request}");
        println!("key-id={key_id} signature={signature}");
        return Ok(());
    }
    let envelope = format!(
        "{{:enrollment/request {} :enrollment/key-id {} :enrollment/signature {}}}\n",
        edn_string(&request),
        edn_string(&key_id),
        edn_string(&signature)
    );
    post("/v1/enrollments", &envelope)
}

fn key_command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("list") => get("/v1/keys", &args[1..]),
        Some("rotate") => post("/v1/keys/rotate", "{}\n"),
        Some("revoke") => {
            let key_id = args.get(1).ok_or("id key revoke requires KEY_ID")?;
            post(
                &format!("/v1/keys/{key_id}/revocations"),
                "{:revocation/reason :publisher-request}\n",
            )
        }
        _ => Err("usage: hara id key <list|rotate|revoke KEY_ID>".into()),
    }
}

pub fn canonical_enrollment(tap: &str, owner: &str, public_key: &str, challenge: &str) -> String {
    format!(
        "{{:enrollment/format \"0.0.0-alpha\" :enrollment/tap {} :enrollment/provider :github :enrollment/owner {} :enrollment/public-key {} :enrollment/challenge {}}}\n",
        edn_string(tap),
        edn_string(owner),
        edn_string(public_key),
        edn_string(challenge)
    )
}

fn fetch_challenge(owner: &str) -> Result<String, String> {
    let url = format!(
        "{}/v1/enrollments/challenge?owner={owner}",
        endpoint().trim_end_matches('/')
    );
    let output = Command::new("curl")
        .args(["--fail-with-body", "--silent", "--show-error", &url])
        .output()
        .map_err(|error| format!("cannot start identity client: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "identity challenge failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let challenge = String::from_utf8(output.stdout)
        .map_err(|_| "identity challenge must be UTF-8")?
        .trim()
        .to_owned();
    if challenge.is_empty() {
        return Err("identity service returned an empty challenge".into());
    }
    Ok(challenge)
}

fn get(path: &str, _args: &[String]) -> Result<(), String> {
    request("GET", path, None)
}

fn post(path: &str, body: &str) -> Result<(), String> {
    request("POST", path, Some(body))
}

fn request_json(
    method: &str,
    path: &str,
    body: Option<JsonValue>,
    bearer: Option<&str>,
) -> Result<JsonValue, String> {
    let url = format!("{}{}", endpoint().trim_end_matches('/'), path);
    let mut command = Command::new("curl");
    command.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "-X",
        method,
        "-H",
        "accept: application/json",
    ]);
    if let Some(secret) = bearer {
        command.args(["-H", &format!("authorization: Bearer {secret}")]);
    }
    let encoded;
    if let Some(body) = body {
        encoded = serde_json::to_string(&body)
            .map_err(|error| format!("cannot encode identity request: {error}"))?;
        command.args([
            "-H",
            "content-type: application/json",
            "--data-binary",
            &encoded,
        ]);
    }
    let output = command
        .arg(url)
        .output()
        .map_err(|error| format!("cannot start identity client: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "identity request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("identity service returned invalid JSON: {error}"))
}

fn request(method: &str, path: &str, body: Option<&str>) -> Result<(), String> {
    let url = format!("{}{}", endpoint().trim_end_matches('/'), path);
    let mut command = Command::new("curl");
    command.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "-X",
        method,
        "-H",
        "accept: application/edn",
    ]);
    if let Some(body) = body {
        command.args(["-H", "content-type: application/edn", "--data-binary", body]);
    }
    let output = command
        .arg(url)
        .output()
        .map_err(|error| format!("cannot start identity client: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "identity request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

fn endpoint() -> String {
    env::var("HARA_ID_ENDPOINT").unwrap_or_else(|_| DEFAULT_IDENTITY_ENDPOINT.into())
}

fn required_option(args: &[String], name: &str) -> Result<String, String> {
    optional_option(args, name).ok_or_else(|| format!("id enroll requires {name}"))
}

fn optional_option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|value| value == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn validate_hex(value: &str, bytes: usize, label: &str) -> Result<(), String> {
    if value.len() != bytes * 2
        || !value
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
    {
        return Err(format!("{label} must be lowercase {}-byte hex", bytes));
    }
    Ok(())
}

fn edn_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

fn usage() {
    println!("hara id login");
    println!("hara id enroll --owner OWNER [--tap hara] [--dry-run]");
    println!("hara id policy grant --identity PATH --root-key-file PATH --key-id ID --public-key HEX --github-subject ID --coordinate COORDINATE --authorization-public-key HEX [--dry-run]");
    println!("hara id status");
    println!("hara id namespace");
    println!("hara id key <list|rotate|revoke KEY_ID>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn enrollment_bytes_are_stable_and_bind_the_public_key() {
        assert_eq!(
            canonical_enrollment(
                "hara",
                "alice",
                &"ab".repeat(32),
                "challenge-1"
            ),
            format!(
                "{{:enrollment/format \"0.0.0-alpha\" :enrollment/tap \"hara\" :enrollment/provider :github :enrollment/owner \"alice\" :enrollment/public-key \"{}\" :enrollment/challenge \"challenge-1\"}}\n",
                "ab".repeat(32)
            )
        );
    }

    #[test]
    fn publisher_device_proof_binds_id_challenge_and_mode() {
        assert_eq!(
            publisher_proof_message("device-1", "challenge-1", "grant"),
            "hara-publisher-device/1\ndevice-1\nchallenge-1\ngrant\n"
        );
        assert_ne!(
            publisher_proof_message("device-1", "challenge-1", "grant"),
            publisher_proof_message("device-1", "challenge-1", "authorize")
        );
    }

    #[test]
    fn policy_grant_is_exact_idempotent_and_root_verifiable() {
        let root = SigningKey::from_bytes(&[9; 32]);
        let root_public = root
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let source = format!(
            "{{:identity/format 1 :identity/root-key \"{root_public}\" :publisher-keys {{}}}}\n"
        );
        let publisher = "ab".repeat(32);
        let authorization = "cd".repeat(32);
        let updated = policy_with_grant(
            &source,
            &root_public,
            "hoebat-2026-01",
            &publisher,
            "1455572",
            "hara:hara-native/smoke-answer",
            &authorization,
        )
        .unwrap();
        assert!(updated.contains(":github-subject \"1455572\""));
        assert!(updated.contains(":coordinates [\"hara:hara-native/smoke-answer\"]"));
        assert!(updated.contains(":identity/publish-authorization-key"));
        let signature = root.sign(updated.as_bytes());
        verify_root_signature(
            &root_public,
            updated.as_bytes(),
            &signature
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        )
        .unwrap();
        assert_eq!(
            policy_with_grant(
                &updated,
                &root_public,
                "hoebat-2026-01",
                &publisher,
                "1455572",
                "hara:hara-native/smoke-answer",
                &authorization,
            )
            .unwrap(),
            updated
        );
        assert!(policy_with_grant(
            &updated,
            &root_public,
            "hoebat-2026-01",
            &"ef".repeat(32),
            "1455572",
            "hara:hara-native/smoke-answer",
            &authorization,
        )
        .is_err());
    }
}
