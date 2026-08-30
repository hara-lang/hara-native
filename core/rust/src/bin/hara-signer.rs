//! Development-only signer commands for the `hara-native` executable.
//!
//! The empty signer-command mode is the `HARA_SIGNER` protocol: it reads the
//! canonical intent from stdin and writes one EDN response to stdout. Key
//! generation and public-key inspection are explicit subcommands so signing
//! can never create or overwrite key material as a side effect.

use ed25519_dalek::{Signer, SigningKey};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const KEY_BYTES: usize = 32;
const SIGNER_KEY_FILE: &str = "HARA_SIGNER_KEY_FILE";
const SIGNER_KEY_ID: &str = "HARA_SIGNER_KEY_ID";

fn usage() -> &'static str {
    r#"Usage:
  hara-native signer generate --key-file ABSOLUTE_PATH
  hara-native signer public-key --key-file ABSOLUTE_PATH
  HARA_SIGNER_KEY_FILE=ABSOLUTE_PATH HARA_SIGNER_KEY_ID=KEY_ID hara-native signer sign < intent.edn

hara-native stores a development Ed25519 seed in a new 0600 Unix file.
It is not an HSM or keychain signer."#
}

pub fn is_configured() -> bool {
    env::var_os(SIGNER_KEY_FILE).is_some()
}

pub fn run(arguments: Vec<String>) -> Result<(), String> {
    match arguments.as_slice() {
        [] => sign_stdin(),
        [command] if command == "sign" => sign_stdin(),
        [command] if command == "--help" || command == "-h" => {
            println!("{}", usage());
            Ok(())
        }
        [command, option, value] if command == "generate" && option == "--key-file" => {
            generate(Path::new(value))
        }
        [command, option, value] if command == "public-key" && option == "--key-file" => {
            println!("{}", public_key_hex(Path::new(value))?);
            Ok(())
        }
        _ => Err(usage().to_owned()),
    }
}

fn sign_stdin() -> Result<(), String> {
    let (key_id, signature) = sign_intent_from_environment(&read_intent()?)?;
    println!("{{:key/id \"{key_id}\" :signature \"{signature}\"}}");
    Ok(())
}

pub fn sign_intent_from_environment(intent: &[u8]) -> Result<(String, String), String> {
    let key_path = required_absolute_path(SIGNER_KEY_FILE)?;
    let key_id = required_key_id()?;
    let signature = sign_with_key_file(&key_path, intent)?;
    Ok((key_id, hex(&signature)))
}

pub fn public_key_from_environment() -> Result<String, String> {
    public_key_hex(&required_absolute_path(SIGNER_KEY_FILE)?)
}

fn read_intent() -> Result<Vec<u8>, String> {
    let mut intent = Vec::new();
    io::stdin()
        .read_to_end(&mut intent)
        .map_err(|error| format!("cannot read publication intent: {error}"))?;
    if intent.is_empty() {
        return Err("refusing to sign an empty publication intent".into());
    }
    Ok(intent)
}

fn generate(path: &Path) -> Result<(), String> {
    let path = absolute_path(path)?;
    let mut seed = [0_u8; KEY_BYTES];
    getrandom::getrandom(&mut seed)
        .map_err(|error| format!("cannot generate Ed25519 seed: {error}"))?;
    let public_key = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let result = write_new_private_seed(&path, &seed);
    seed.zeroize();
    result?;
    println!("created {}", path.display());
    println!("public-key {}", hex(&public_key));
    Ok(())
}

/// Derives the lowercase Ed25519 public key for an explicitly selected local
/// seed file. The policy-maintainer command uses this to prove that the
/// supplied offline root key matches the policy before it writes anything.
pub fn public_key_hex(path: &Path) -> Result<String, String> {
    let mut seed = read_private_seed(&absolute_path(path)?)?;
    let public_key = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    seed.zeroize();
    Ok(hex(&public_key))
}

/// Signs exact bytes with an explicitly selected local seed file. This is
/// separate from publisher environment variables so an offline identity root
/// key cannot be used accidentally for publication.
pub fn sign_with_key_file(path: &Path, intent: &[u8]) -> Result<[u8; 64], String> {
    let mut seed = read_private_seed(path)?;
    let signing_key = SigningKey::from_bytes(&seed);
    seed.zeroize();
    Ok(signing_key.sign(intent).to_bytes())
}

fn required_absolute_path(name: &str) -> Result<PathBuf, String> {
    let value =
        env::var(name).map_err(|_| format!("{name} must name an absolute key file path"))?;
    absolute_path(Path::new(&value))
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(format!(
            "key file path must be absolute: {}",
            path.display()
        ))
    }
}

fn required_key_id() -> Result<String, String> {
    let key_id = env::var(SIGNER_KEY_ID)
        .map_err(|_| format!("{SIGNER_KEY_ID} must name the enrolled publisher key"))?;
    validate_key_id(&key_id)?;
    Ok(key_id)
}

fn validate_key_id(key_id: &str) -> Result<(), String> {
    if key_id.is_empty()
        || !key_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')
        })
    {
        return Err(format!(
            "{SIGNER_KEY_ID} must contain only ASCII letters, digits, '-', '_', '.', '/', or ':'"
        ));
    }
    Ok(())
}

fn read_private_seed(path: &Path) -> Result<[u8; KEY_BYTES], String> {
    check_private_file(path)?;
    let mut bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() != KEY_BYTES {
        bytes.zeroize();
        return Err(format!(
            "{} must contain exactly {KEY_BYTES} private-key bytes",
            path.display()
        ));
    }
    let mut seed = [0_u8; KEY_BYTES];
    seed.copy_from_slice(&bytes);
    bytes.zeroize();
    Ok(seed)
}

#[cfg(unix)]
fn write_new_private_seed(path: &Path, seed: &[u8; KEY_BYTES]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path
        .parent()
        .ok_or_else(|| format!("key file has no parent directory: {}", path.display()))?;
    if !parent.is_dir() {
        return Err(format!(
            "key file parent directory does not exist: {}",
            parent.display()
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("cannot create private key {}: {error}", path.display()))?;
    file.write_all(seed)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("cannot write private key {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn write_new_private_seed(_path: &Path, _seed: &[u8; KEY_BYTES]) -> Result<(), String> {
    Err(
        "hara-native signer development keys require a Unix filesystem with 0600 permissions"
            .into(),
    )
}

#[cfg(unix)]
fn check_private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect private key {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(format!(
            "private key must be a regular file: {}",
            path.display()
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "private key must not be group- or world-accessible: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_private_file(_path: &Path) -> Result<(), String> {
    Err(
        "hara-native signer development keys require a Unix filesystem with 0600 permissions"
            .into(),
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, VerifyingKey};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_PATH: AtomicUsize = AtomicUsize::new(0);

    fn temporary_key_path() -> PathBuf {
        let suffix = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let root =
            env::temp_dir().join(format!("hara-signer-test-{}-{suffix}", std::process::id()));
        fs::create_dir(&root).unwrap();
        root.join("publisher.ed25519")
    }

    #[test]
    fn generated_seed_derives_the_reported_public_key() {
        let path = temporary_key_path();
        generate(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
        let seed = read_private_seed(&path).unwrap();
        let expected = hex(&SigningKey::from_bytes(&seed).verifying_key().to_bytes());
        assert_eq!(public_key_hex(&path).unwrap(), expected);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn signatures_verify_against_the_derived_public_key() {
        let path = temporary_key_path();
        let seed = [7_u8; KEY_BYTES];
        write_new_private_seed(&path, &seed).unwrap();
        let intent = b"{:intent/format \"0.0.0-alpha\"}\n";
        let signature = Signature::from_slice(&sign_with_key_file(&path, intent).unwrap()).unwrap();
        let public_key =
            VerifyingKey::from_bytes(&SigningKey::from_bytes(&seed).verifying_key().to_bytes())
                .unwrap();
        assert!(public_key.verify_strict(intent, &signature).is_ok());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn signer_key_ids_cannot_escape_the_edn_response() {
        assert!(validate_key_id("publisher-2026").is_ok());
        assert!(validate_key_id("publisher\"bad").is_err());
        assert!(validate_key_id("publisher bad").is_err());
    }
}
