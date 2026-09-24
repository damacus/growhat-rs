use serde_json::Value;
use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_growhat"))
}

#[test]
fn diagnostic_is_bounded_ndjson_without_mqtt() {
    let result = binary()
        .args([
            "--config",
            "examples/simulated.toml",
            "diagnose",
            "--samples",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let stdout = String::from_utf8(result.stdout).unwrap();
    let rows: Vec<Value> = stdout
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "uncalibrated");
    assert_eq!(rows[0]["raw_hz"], 10.0);
    assert!(rows[0]["moisture_percent"].is_null());
}

#[test]
fn hardware_requires_confirmation_before_accessing_devices() {
    let result = binary()
        .args([
            "--config",
            "examples/hardware.toml",
            "diagnose",
            "--samples",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("hardware_confirmed"));
}

#[test]
fn zero_samples_is_a_usage_error() {
    let result = binary()
        .args([
            "--config",
            "examples/simulated.toml",
            "diagnose",
            "--samples",
            "0",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
}

#[test]
fn version_does_not_need_configuration_or_hardware() {
    let result = binary().arg("--version").output().unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).starts_with("growhat "));
}
