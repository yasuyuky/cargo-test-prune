use std::{collections::BTreeSet, path::Path, process::Command, time::Duration};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{
    Analyze,
    matrix::{Outcome, Target, Test},
    process::{self, Output},
    workspace::{self, Workspace},
};

pub struct Runner<'a> {
    pub workspace: &'a Workspace,
    pub options: &'a Analyze,
}

impl Runner<'_> {
    pub fn command(&self) -> Command {
        let mut command = workspace::cargo();
        command.current_dir(&self.workspace.root);
        command
    }

    pub fn features(&self, command: &mut Command) {
        if !self.options.features.is_empty() {
            command
                .arg("--features")
                .arg(self.options.features.join(","));
        }
        if self.options.all_features {
            command.arg("--all-features");
        }
        if self.options.no_default_features {
            command.arg("--no-default-features");
        }
    }

    fn test_command(&self, target: Option<&Target>) -> Command {
        let mut command = self.command();
        command
            .args(["test", "--manifest-path"])
            .arg(self.workspace.root.join("Cargo.toml"));
        command
            .arg("--target-dir")
            .arg(self.workspace.root.join("target"));
        if let Some(target) = target {
            command.args(["-p", &target.package]);
            if target.kind == "lib" {
                command.arg("--lib");
            } else {
                command.arg(format!("--{}", target.kind)).arg(&target.name);
            }
        } else {
            command.arg("--tests");
            if let Some(package) = &self.options.package {
                command.args(["-p", package]);
            } else {
                command.arg("--workspace");
            }
        }
        self.features(&mut command);
        command
    }

    pub fn build(&self) -> Result<Output> {
        process::run(
            self.test_command(None)
                .args(["--no-run", "--message-format=json"]),
            Duration::from_secs(self.options.build_timeout),
        )
    }

    pub fn discover(&self, build: &Output) -> Result<Vec<Test>> {
        let mut targets = BTreeSet::new();
        for line in build.stdout.lines() {
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if message["reason"] != "compiler-artifact"
                || message["profile"]["test"] != true
                || message["executable"].is_null()
            {
                continue;
            }
            let package = self.workspace.metadata["packages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|package| package["id"] == message["package_id"])
                .context("unknown test package")?;
            let kinds = message["target"]["kind"]
                .as_array()
                .context("missing target kind")?;
            let kind = if kinds.iter().any(|k| k == "test") {
                "test"
            } else if kinds.iter().any(|k| k == "bin") {
                "bin"
            } else {
                "lib"
            };
            targets.insert(Target {
                package: package["name"]
                    .as_str()
                    .context("missing package name")?
                    .to_owned(),
                kind: kind.to_owned(),
                name: message["target"]["name"]
                    .as_str()
                    .context("missing target name")?
                    .to_owned(),
            });
        }
        let mut tests = Vec::new();
        for target in targets {
            ensure_libtest(&self.workspace.root, &self.workspace.metadata, &target)?;
            let output = process::run(
                self.test_command(Some(&target))
                    .args(["--", "--list", "--format=terse"]),
                Duration::from_secs(self.options.build_timeout),
            )?;
            ensure!(
                output.success(),
                "test discovery failed for {}:\n{}",
                target.name,
                output.diagnostic()
            );
            for line in output.stdout.lines().filter(|line| !line.is_empty()) {
                if line.ends_with(": benchmark") {
                    continue;
                }
                let name = line
                    .strip_suffix(": test")
                    .context("unsupported test harness output; expected libtest --list")?;
                tests.push(Test {
                    target: target.clone(),
                    name: name.to_owned(),
                    baseline: Outcome::Unknown,
                    duration_ms: 0,
                });
            }
        }
        tests.sort_by_key(Test::id);
        ensure!(
            !tests.is_empty(),
            "no libtest tests found for the selected packages and features"
        );
        Ok(tests)
    }

    pub fn test(&self, test: &Test) -> Result<Output> {
        process::run(
            self.test_command(Some(&test.target)).args([
                "--",
                "--exact",
                &test.name,
                "--test-threads=1",
                "--format=terse",
            ]),
            Duration::from_secs(self.options.timeout),
        )
    }
}

fn ensure_libtest(root: &Path, metadata: &Value, target: &Target) -> Result<()> {
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == target.package)
        .unwrap();
    let original = Path::new(package["manifest_path"].as_str().unwrap());
    let relative = original.strip_prefix(metadata["workspace_root"].as_str().unwrap())?;
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(root.join(relative))?)?;
    let custom =
        |value: &toml::Value| value.get("harness").and_then(toml::Value::as_bool) == Some(false);
    let unsupported = if target.kind == "lib" {
        manifest.get("lib").is_some_and(custom)
    } else {
        manifest
            .get(&target.kind)
            .and_then(toml::Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("name").and_then(toml::Value::as_str) == Some(&target.name)
                        && custom(item)
                })
            })
    };
    ensure!(
        !unsupported,
        "custom test harnesses are unsupported: {}",
        target.name
    );
    Ok(())
}

pub fn outcome(output: &Output) -> Outcome {
    let Some(status) = output.status else {
        return Outcome::Timeout;
    };
    let Some(summary) = output
        .stdout
        .lines()
        .rev()
        .find(|line| line.starts_with("test result: "))
    else {
        return Outcome::Unknown;
    };
    let counts: Option<Vec<usize>> = ["passed", "failed", "ignored", "measured"]
        .iter()
        .map(|label| {
            let words: Vec<_> = summary.split_whitespace().collect();
            let index = words
                .iter()
                .position(|word| word.trim_end_matches(';') == *label)?;
            words.get(index.checked_sub(1)?)?.parse().ok()
        })
        .collect();
    match counts.as_deref() {
        Some([1, 0, 0, 0]) if status.success() => Outcome::Passed,
        Some([0, 1, 0, 0]) if status.code() == Some(101) => Outcome::Killed,
        Some([0, 0, 1, 0]) if status.success() => Outcome::Ignored,
        _ => Outcome::Unknown,
    }
}
