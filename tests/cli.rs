use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::create_dir(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        include_str!("fixtures/overlap/Cargo.toml"),
    )
    .unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        include_str!("fixtures/overlap/src/lib.rs"),
    )
    .unwrap();
    fs::write(
        dir.path().join("tests/integration.rs"),
        include_str!("fixtures/overlap/tests/integration.rs"),
    )
    .unwrap();
    dir
}

fn mutants(dir: &Path, replacements: &[&str]) -> std::path::PathBuf {
    let mutations: Vec<_> = replacements
        .iter()
        .map(|replacement| {
            json!({
                "name": format!("replace body with {replacement}"), "file": "src/lib.rs",
                "span": {"start": {"line": 2, "column": 5}, "end": {"line": 2, "column": 14}},
                "replacement": replacement,
            })
        })
        .collect();
    let path = dir.join("mutants.json");
    fs::write(&path, serde_json::to_string(&mutations).unwrap()).unwrap();
    path
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-test-prune"))
        .current_dir(dir)
        .env("CARGO_NET_OFFLINE", "true")
        .arg("test-prune")
        .args(args)
        .output()
        .unwrap()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn measures_actual_tests_and_keeps_the_detection_union() {
    let dir = fixture();
    mutants(
        dir.path(),
        &["true", "false", "value >= 0", "not_a_valid_expression"],
    );
    let output = run(dir.path(), &["analyze", "--mutants-file", "mutants.json"]);
    success(&output);
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("test-prune.json")).unwrap())
            .unwrap();
    assert_eq!(report["analysis"]["tests"].as_array().unwrap().len(), 6);
    assert_eq!(report["analysis"]["mutants"][3]["status"], "unviable");
    assert_eq!(report["recommendation"]["detected_mutants"], 3);
    let candidates = report["recommendation"]["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(
        candidates
            .iter()
            .all(|t| !t["test"].as_str().unwrap().ends_with("::zero")
                && !t["test"].as_str().unwrap().ends_with("::ignored"))
    );
    let retained = report["recommendation"]["retained"].as_array().unwrap();
    let remaining: BTreeSet<_> = retained
        .iter()
        .flat_map(|t| t["killed_mutants"].as_array().unwrap())
        .map(|m| m.as_u64().unwrap())
        .collect();
    assert_eq!(remaining, BTreeSet::from([1, 2, 3]));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        include_str!("fixtures/overlap/src/lib.rs")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tests/integration.rs")).unwrap(),
        include_str!("fixtures/overlap/tests/integration.rs")
    );
    let replay = run(
        dir.path(),
        &["report", "test-prune.json", "--keep", "positive_two"],
    );
    success(&replay);
    assert!(String::from_utf8_lossy(&replay.stdout).contains("positive_two — explicitly kept"));
}

#[test]
fn workspace_scope_features_and_package_selection_are_respected() {
    let dir = fixture();
    let manifest = include_str!("fixtures/overlap/Cargo.toml").replace(
        "[workspace]",
        "[features]\nextra = []\n\n[workspace]\nmembers = [\"consumer\"]\nresolver = \"3\"",
    );
    fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();
    fs::create_dir_all(dir.path().join("consumer/src")).unwrap();
    fs::write(dir.path().join("consumer/Cargo.toml"), "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\noverlap = { path = \"..\" }\n").unwrap();
    fs::write(
        dir.path().join("consumer/src/lib.rs"),
        "#[test]\nfn positive_one() { assert!(overlap::positive(1)); }\n",
    )
    .unwrap();
    let mut source = include_str!("fixtures/overlap/src/lib.rs").to_owned();
    source.push_str(
        "\n#[cfg(feature = \"extra\")]\n#[test]\nfn extra_test() { assert!(positive(1)); }\n",
    );
    fs::write(dir.path().join("src/lib.rs"), source).unwrap();
    mutants(dir.path(), &["false"]);
    let output = run(
        dir.path(),
        &[
            "analyze",
            "--mutants-file",
            "mutants.json",
            "--features",
            "overlap/extra",
        ],
    );
    success(&output);
    let read_tests = || {
        let report: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("test-prune.json")).unwrap())
                .unwrap();
        report["analysis"]["tests"].as_array().unwrap().clone()
    };
    let tests = read_tests();
    assert_eq!(tests.len(), 8);
    assert!(tests.iter().any(|t| t["target"]["package"] == "consumer"));
    assert!(tests.iter().any(|t| t["name"] == "extra_test"));
    let selected = run(
        dir.path(),
        &[
            "analyze",
            "--mutants-file",
            "mutants.json",
            "-p",
            "overlap",
            "--apply",
        ],
    );
    success(&selected);
    let tests = read_tests();
    assert_eq!(tests.len(), 6);
    assert!(tests.iter().all(|t| t["target"]["package"] == "overlap"));
    assert!(
        fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .contains("fn extra_test()")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("consumer/src/lib.rs")).unwrap(),
        "#[test]\nfn positive_one() { assert!(overlap::positive(1)); }\n"
    );
}
