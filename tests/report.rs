use std::{fs, process::Command};

use serde_json::{Value, json};

fn report() -> Value {
    let rows = [vec![0, 1], vec![1, 2], vec![0, 2], vec![0], vec![3], vec![]];
    let tests: Vec<_> = ["a", "b", "c", "d", "unrelated", "no_kills"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            json!({
                "target": {"package": "demo", "kind": "lib", "name": "demo"},
                "name": name, "baseline": "passed",
                "duration_ms": if i < 2 { 100 } else { 1 },
            })
        })
        .collect();
    let mutants: Vec<_> = (0..4)
        .map(|i| {
            let outcomes: Vec<_> = rows
                .iter()
                .map(|row| if row.contains(&i) { "killed" } else { "passed" })
                .collect();
            json!({
                "mutant": {
                    "name": format!("replace condition {}", i + 1), "file": "src/lib.rs",
                    "span": {"start": {"line": 1, "column": 1}, "end": {"line": 1, "column": 2}},
                    "replacement": "0",
                },
                "status": "tested", "outcomes": outcomes, "diagnostic": null,
            })
        })
        .collect();
    json!({
        "analysis": {
            "schema_version": 1, "workspace": "/unavailable/workspace", "package": null,
            "features": [], "all_features": false, "no_default_features": false,
            "generated_mutants": 4, "tests": tests, "mutants": mutants,
        },
        "recommendation": {
            "weight": "count", "detected_mutants": 0, "retained": [], "candidates": [],
        },
    })
}

fn render(report: &Value, args: &[&str]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test-prune.json");
    let saved = serde_json::to_string(report).unwrap();
    fs::write(&path, &saved).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-test-prune"))
        .current_dir(dir.path())
        .env("CARGO", dir.path().join("missing-cargo"))
        .args(["test-prune", "report", "test-prune.json"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(path).unwrap(), saved);
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn mermaid_shows_each_candidates_mutants_and_retained_cover() {
    let output = render(&report(), &["--format", "mermaid"]);
    let diagrams: Vec<_> = output
        .split("```mermaid\n")
        .skip(1)
        .map(|part| part.split("```\n").next().unwrap())
        .collect();
    assert_eq!(diagrams.len(), 2);
    let first = diagrams[0];
    assert!(first.contains("Candidate: demo/lib:demo/c"));
    assert!(first.contains("Retain: demo/lib:demo/a"));
    assert!(first.contains("Retain: demo/lib:demo/b"));
    assert!(first.contains("M1: replace condition 1"));
    assert!(first.contains("M3: replace condition 3"));
    assert!(first.contains("candidate -.-> mutant_1"));
    assert!(first.contains("candidate -.-> mutant_3"));
    assert!(first.contains("retained_0 --> mutant_1"));
    assert!(first.contains("retained_1 --> mutant_3"));
    assert!(!first.contains("retained_0 --> mutant_3"));
    assert!(!first.contains("retained_1 --> mutant_1"));
    assert!(!first.contains("mutant_2"));
    assert!(!first.contains("mutant_4"));
    let second = diagrams[1];
    assert!(second.contains("Candidate: demo/lib:demo/d"));
    assert!(second.contains("Retain: demo/lib:demo/a"));
    assert!(second.contains("retained_0 --> mutant_1"));
    assert!(!second.contains("Retain: demo/lib:demo/b"));
    assert!(!second.contains("mutant_3"));
    assert!(!output.contains("unrelated"));
    assert!(!output.contains("no_kills"));
}

#[test]
fn mermaid_recomputes_selection_with_pins_and_weights() {
    for selection in [["--keep", "/c"], ["--weight", "time"]] {
        let output = render(
            &report(),
            &["--format", "mermaid", selection[0], selection[1]],
        );
        assert!(output.contains("Candidate: demo/lib:demo/b"));
        assert!(!output.contains("Candidate: demo/lib:demo/c"));
        assert!(output.contains("Retain: demo/lib:demo/c"));
    }
}

#[test]
fn mermaid_handles_no_candidates() {
    let output = render(&report(), &["--format", "mermaid", "--keep", "demo"]);
    assert!(output.contains("No removal candidates."));
    assert!(!output.contains("```mermaid"));
}

#[test]
fn mermaid_escapes_labels_without_breaking_diagrams() {
    let mut evidence = report();
    evidence["analysis"]["tests"][2]["name"] = json!("quoted\"[name]\\path\n```mermaid");
    evidence["analysis"]["tests"][0]["name"] = json!("kept<test>");
    evidence["analysis"]["mutants"][0]["mutant"]["name"] =
        json!("replace \"x\" < y && #quot; > `値`\r\nnext");
    let output = render(&evidence, &["--format", "mermaid"]);
    assert!(output.contains("quoted#34;[name]#92;path #96;#96;#96;mermaid"));
    assert!(output.contains("Retain: demo/lib:demo/kept#60;test#62;"));
    assert!(
        output.contains("M1: replace #34;x#34; #60; y #38;#38; #35;quot; #62; #96;値#96;  next")
    );
    assert_eq!(output.matches("```mermaid\n").count(), 2);
    assert!(!output.contains("<test>"));
}

#[test]
fn text_remains_the_default_format() {
    let output = render(&report(), &[]);
    assert_eq!(output, render(&report(), &["--format", "text"]));
    assert!(output.contains("Removal candidates (one combined proposal):"));
    assert!(!output.contains("```mermaid"));
}
