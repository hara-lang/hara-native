//! Coordinator and evidence validator for native VM/JIT/whole-Wasm benchmarks.
//!
//! Feature-specific workers are built separately. This binary deliberately
//! compares rows only within one invocation, records worker digests, and never
//! treats a benchmark result as a cross-host or historical performance claim.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const EVIDENCE_SCHEMA: &str = "hara.native-benchmark/v1";
const CORPUS_SCHEMA: &str = "hara.native-benchmark-corpus/v1";
const RULES_SCHEMA: &str = "hara.native-benchmark-rules/v1";

#[derive(Clone, Copy)]
struct Profile {
    name: &'static str,
    windows: usize,
    smoke_calls: bool,
}

fn profile(value: &str) -> Result<Profile, String> {
    match value {
        "smoke" => Ok(Profile {
            name: "smoke",
            windows: 3,
            smoke_calls: true,
        }),
        "guard" => Ok(Profile {
            name: "guard",
            windows: 15,
            smoke_calls: false,
        }),
        "standard" => Ok(Profile {
            name: "standard",
            windows: 60,
            smoke_calls: false,
        }),
        _ => Err("profile must be smoke, guard, or standard".to_owned()),
    }
}

fn usage() -> &'static str {
    "hara-native-benchmark run --profile PROFILE --corpus PATH --rules PATH --output PATH \\
  --vm PATH --trace-checked PATH --trace-native PATH --whole-wasm PATH\n\
hara-native-benchmark validate --evidence PATH --rules PATH"
}

fn options(
    arguments: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let mut options = BTreeMap::new();
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        if !flag.starts_with("--") {
            return Err(format!("expected an option, received {flag}"));
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if options.insert(flag, value).is_some() {
            return Err("benchmark options must not repeat".to_owned());
        }
    }
    Ok(options)
}

fn required<'a>(options: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, String> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing required option {name}"))
}

fn read_json(path: &str) -> Result<Value, String> {
    let source = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    serde_json::from_str(&source).map_err(|error| format!("{path}: {error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn source_hex(source: &str) -> String {
    source
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn command_output(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| error.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("worker exited {}: {stderr}", output.status))
    }
}

fn worker_identity(path: &str) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read worker {path}: {error}"))?;
    Ok(json!({
        "path": path,
        "bytes": bytes.len(),
        "sha256": sha256_hex(&bytes),
    }))
}

fn worker_row(
    worker: &str,
    tier: &str,
    workload: &Value,
    profile: Profile,
) -> Result<Value, String> {
    let id = required_value_string(workload, "id")?;
    let source = required_value_string(workload, "hara_source")?;
    let expected = required_value_string(workload, "expected")?;
    let calls = if profile.smoke_calls {
        1
    } else {
        workload["calls_per_window"]
            .as_u64()
            .ok_or_else(|| format!("{id}: calls_per_window must be an unsigned integer"))?
    };
    let stdout = command_output(
        Command::new(worker)
            .arg(tier)
            .arg(id)
            .arg(source_hex(source))
            .arg(expected)
            .arg(profile.windows.to_string())
            .arg(calls.to_string()),
    )?;
    let mut row: Value = serde_json::from_str(stdout.trim())
        .map_err(|error| format!("{tier}/{id}: worker returned invalid JSON: {error}"))?;
    let object = row
        .as_object_mut()
        .ok_or_else(|| format!("{tier}/{id}: worker result must be an object"))?;
    object.insert("group".to_owned(), workload["group"].clone());
    object.insert(
        "source_sha256".to_owned(),
        Value::String(sha256_hex(source.as_bytes())),
    );
    Ok(row)
}

fn required_value_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("workload {key} must be a string"))
}

fn validate_corpus(corpus: &Value) -> Result<(), String> {
    if corpus["schema"].as_str() != Some(CORPUS_SCHEMA) {
        return Err(format!("corpus schema must be {CORPUS_SCHEMA}"));
    }
    let workloads = corpus["workloads"]
        .as_array()
        .ok_or_else(|| "corpus workloads must be an array".to_owned())?;
    if workloads.is_empty() {
        return Err("corpus must contain at least one workload".to_owned());
    }
    let mut ids = BTreeSet::new();
    for workload in workloads {
        let id = required_value_string(workload, "id")?;
        if !ids.insert(id) {
            return Err(format!("corpus repeats workload {id}"));
        }
        match required_value_string(workload, "group")? {
            "portable" | "diagnostic" => {}
            group => return Err(format!("{id}: unsupported benchmark group {group}")),
        }
        required_value_string(workload, "hara_source")?;
        required_value_string(workload, "expected")?;
        if workload["calls_per_window"]
            .as_u64()
            .filter(|calls| *calls > 0)
            .is_none()
        {
            return Err(format!("{id}: calls_per_window must be positive"));
        }
    }
    Ok(())
}

fn analysis(samples: &[u64]) -> Result<Value, String> {
    if samples.is_empty() {
        return Err("measurement samples must not be empty".to_owned());
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let count = sorted.len();
    let p50 = sorted[count / 2];
    let p95 = sorted[(count * 95).div_ceil(100).saturating_sub(1)];
    let mean = samples.iter().map(|value| *value as f64).sum::<f64>() / count as f64;
    let variance = samples
        .iter()
        .map(|value| {
            let delta = *value as f64 - mean;
            delta * delta
        })
        .sum::<f64>()
        / count as f64;
    let cv_ppm = if mean == 0.0 {
        0
    } else {
        (variance.sqrt() / mean * 1_000_000.0).round() as u64
    };
    Ok(json!({
        "min_ns": sorted[0],
        "p50_ns": p50,
        "p95_ns": p95,
        "max_ns": sorted[count - 1],
        "mean_ns": mean.round() as u64,
        "cv_ppm": cv_ppm,
    }))
}

fn enrich_analysis(row: &mut Value) -> Result<(), String> {
    let samples = row["samples_ns"]
        .as_array()
        .ok_or_else(|| "worker samples_ns must be an array".to_owned())?
        .iter()
        .map(|sample| {
            sample
                .as_u64()
                .ok_or_else(|| "sample must be an unsigned integer".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    row.as_object_mut()
        .ok_or_else(|| "worker result must be an object".to_owned())?
        .insert("analysis".to_owned(), analysis(&samples)?);
    Ok(())
}

fn platform_value() -> Value {
    let revision = env::var("GITHUB_SHA").ok().or_else(|| {
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
    });
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned());
    json!({
        "generated_at_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "available_parallelism": std::thread::available_parallelism().map(|count| count.get()).ok(),
        "git_revision": revision,
        "rustc": rustc,
        "ci": env::var("CI").ok(),
        "container_image": env::var("HARA_BENCH_CONTAINER_IMAGE").ok(),
    })
}

fn p50(row: &Value) -> Result<f64, String> {
    row["analysis"]["p50_ns"]
        .as_u64()
        .map(|value| value as f64)
        .ok_or_else(|| "row is missing analysis.p50_ns".to_owned())
}

fn matching_row<'a>(evidence: &'a Value, tier: &str, workload: &str) -> Result<&'a Value, String> {
    evidence["measurements"]
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["tier"].as_str() == Some(tier) && row["workload"].as_str() == Some(workload)
            })
        })
        .ok_or_else(|| format!("missing measurement for {tier}/{workload}"))
}

fn validate_evidence(evidence: &Value, rules: &Value) -> Result<(), String> {
    if evidence["schema"].as_str() != Some(EVIDENCE_SCHEMA) {
        return Err(format!("evidence schema must be {EVIDENCE_SCHEMA}"));
    }
    if rules["schema"].as_str() != Some(RULES_SCHEMA) {
        return Err(format!("rules schema must be {RULES_SCHEMA}"));
    }
    let profile = profile(
        evidence["profile"]
            .as_str()
            .ok_or_else(|| "evidence profile must be a string".to_owned())?,
    )?;
    let corpus = evidence["corpus"]
        .as_object()
        .ok_or_else(|| "evidence corpus metadata must be an object".to_owned())?;
    let portable = corpus["portable"]
        .as_array()
        .ok_or_else(|| "evidence corpus portable workloads must be an array".to_owned())?;
    let diagnostic = corpus["diagnostic"]
        .as_array()
        .ok_or_else(|| "evidence corpus diagnostic workloads must be an array".to_owned())?;

    for workload in portable {
        let workload = workload
            .as_str()
            .ok_or_else(|| "workload id must be a string".to_owned())?;
        for tier in ["vm", "trace-checked", "trace-native", "whole-wasm"] {
            let row = matching_row(evidence, tier, workload)?;
            if row["status"].as_str() != Some("ok") {
                return Err(format!("{tier}/{workload} did not complete successfully"));
            }
            if row["samples_ns"].as_array().map_or(0, Vec::len) != profile.windows {
                return Err(format!("{tier}/{workload} has the wrong sample count"));
            }
            p50(row)?;
        }
        let whole = matching_row(evidence, "whole-wasm", workload)?;
        if whole["native_entry"].as_bool() != Some(true) {
            return Err(format!("whole-wasm/{workload} did not use a native entry"));
        }
    }
    for workload in diagnostic {
        let workload = workload
            .as_str()
            .ok_or_else(|| "workload id must be a string".to_owned())?;
        for tier in ["vm", "trace-checked", "trace-native"] {
            let row = matching_row(evidence, tier, workload)?;
            if row["status"].as_str() != Some("ok") {
                return Err(format!("{tier}/{workload} did not complete successfully"));
            }
            if row["samples_ns"].as_array().map_or(0, Vec::len) != profile.windows {
                return Err(format!("{tier}/{workload} has the wrong sample count"));
            }
            p50(row)?;
        }
    }

    if profile.name == "smoke" {
        return Ok(());
    }
    for rule in rules["relative_rules"]
        .as_array()
        .ok_or_else(|| "rules relative_rules must be an array".to_owned())?
    {
        let numerator = &rule["numerator"];
        let denominator = &rule["denominator"];
        let numerator_row = matching_row(
            evidence,
            numerator["tier"]
                .as_str()
                .ok_or("numerator tier must be a string")?,
            numerator["workload"]
                .as_str()
                .ok_or("numerator workload must be a string")?,
        )?;
        let denominator_row = matching_row(
            evidence,
            denominator["tier"]
                .as_str()
                .ok_or("denominator tier must be a string")?,
            denominator["workload"]
                .as_str()
                .ok_or("denominator workload must be a string")?,
        )?;
        let ratio = p50(numerator_row)? / p50(denominator_row)?;
        let maximum = rule["max_ratio"]
            .as_f64()
            .ok_or("max_ratio must be numeric")?;
        if ratio > maximum {
            return Err(format!(
                "{} ratio {ratio:.3} exceeds {maximum:.3}",
                rule["id"].as_str().unwrap_or("relative rule")
            ));
        }
    }
    for requirement in rules["required_telemetry"]
        .as_array()
        .ok_or_else(|| "rules required_telemetry must be an array".to_owned())?
    {
        let tier = requirement["tier"]
            .as_str()
            .ok_or("telemetry tier must be a string")?;
        let workload = requirement["workload"]
            .as_str()
            .ok_or("telemetry workload must be a string")?;
        let field = requirement["field"]
            .as_str()
            .ok_or("telemetry field must be a string")?;
        let minimum = requirement["min"]
            .as_u64()
            .ok_or("telemetry min must be an integer")?;
        let actual = matching_row(evidence, tier, workload)?["jit"][field]
            .as_u64()
            .unwrap_or(0);
        if actual < minimum {
            return Err(format!(
                "{tier}/{workload} telemetry {field}={actual} is below {minimum}"
            ));
        }
    }
    Ok(())
}

fn run(options: BTreeMap<String, String>) -> Result<(), String> {
    let profile = profile(required(&options, "--profile")?)?;
    let corpus_path = required(&options, "--corpus")?;
    let rules_path = required(&options, "--rules")?;
    let output_path = required(&options, "--output")?;
    let corpus_bytes = fs::read(corpus_path).map_err(|error| format!("{corpus_path}: {error}"))?;
    let corpus: Value =
        serde_json::from_slice(&corpus_bytes).map_err(|error| format!("{corpus_path}: {error}"))?;
    let rules = read_json(rules_path)?;
    validate_corpus(&corpus)?;
    if rules["schema"].as_str() != Some(RULES_SCHEMA) {
        return Err(format!("rules schema must be {RULES_SCHEMA}"));
    }

    let workers = [
        ("vm", required(&options, "--vm")?),
        ("trace-checked", required(&options, "--trace-checked")?),
        ("trace-native", required(&options, "--trace-native")?),
        ("whole-wasm", required(&options, "--whole-wasm")?),
    ];
    let identities = workers
        .iter()
        .map(|(tier, path)| Ok((tier.to_string(), worker_identity(path)?)))
        .collect::<Result<Map<_, _>, String>>()?;

    let workloads = corpus["workloads"]
        .as_array()
        .ok_or("corpus workloads must be an array")?;
    let mut measurements = Vec::new();
    let mut portable = Vec::new();
    let mut diagnostic = Vec::new();
    for workload in workloads {
        let id = required_value_string(workload, "id")?;
        let group = required_value_string(workload, "group")?;
        match group {
            "portable" => portable.push(Value::String(id.to_owned())),
            "diagnostic" => diagnostic.push(Value::String(id.to_owned())),
            _ => unreachable!("validate_corpus checks benchmark groups"),
        }
        for (tier, worker) in workers {
            if group == "diagnostic" && tier == "whole-wasm" {
                continue;
            }
            let mut row = worker_row(worker, tier, workload, profile)?;
            enrich_analysis(&mut row)?;
            measurements.push(row);
        }
    }
    let evidence = json!({
        "schema": EVIDENCE_SCHEMA,
        "profile": profile.name,
        "protocol": {
            "windows": profile.windows,
            "smoke_calls_per_window": 1,
            "sample_unit": "nanoseconds per prepared call",
            "comparability": "same invocation only",
        },
        "corpus": {
            "id": corpus["id"],
            "sha256": sha256_hex(&corpus_bytes),
            "portable": portable,
            "diagnostic": diagnostic,
        },
        "environment": platform_value(),
        "worker_identities": identities,
        "measurements": measurements,
    });
    validate_evidence(&evidence, &rules)?;
    if let Some(parent) = Path::new(output_path).parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        output_path,
        format!("{}\n", serde_json::to_string_pretty(&evidence).unwrap()),
    )
    .map_err(|error| format!("{output_path}: {error}"))?;
    println!("native benchmark evidence: {output_path}");
    Ok(())
}

fn validate(options: BTreeMap<String, String>) -> Result<(), String> {
    let evidence = read_json(required(&options, "--evidence")?)?;
    let rules = read_json(required(&options, "--rules")?)?;
    validate_evidence(&evidence, &rules)?;
    println!("native benchmark evidence is valid");
    Ok(())
}

fn main() {
    let mut arguments = env::args().skip(1);
    let command = arguments.next();
    let result = match command.as_deref() {
        Some("run") => options(arguments).and_then(run),
        Some("validate") => options(arguments).and_then(validate),
        _ => Err(usage().to_owned()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::{analysis, profile, validate_corpus, CORPUS_SCHEMA};
    use serde_json::json;

    #[test]
    fn profiles_define_the_sampling_protocol() {
        assert_eq!(profile("smoke").unwrap().windows, 3);
        assert_eq!(profile("guard").unwrap().windows, 15);
        assert_eq!(profile("standard").unwrap().windows, 60);
    }

    #[test]
    fn analysis_uses_the_upper_median_and_percentile() {
        let summary = analysis(&[4, 1, 3, 2]).unwrap();
        assert_eq!(summary["p50_ns"], 3);
        assert_eq!(summary["p95_ns"], 4);
    }

    #[test]
    fn corpus_requires_unique_grouped_workloads() {
        let corpus = json!({
            "schema": CORPUS_SCHEMA,
            "workloads": [{
                "id": "one",
                "group": "portable",
                "calls_per_window": 1,
                "hara_source": "(+ 1 1)",
                "expected": "2",
            }],
        });
        assert!(validate_corpus(&corpus).is_ok());
    }
}
