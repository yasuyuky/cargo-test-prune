use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::mutants::Mutant;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Target {
    pub package: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Test {
    pub target: Target,
    pub name: String,
    pub baseline: Outcome,
    pub duration_ms: u64,
}

impl Test {
    pub fn id(&self) -> String {
        format!(
            "{}/{}:{}/{}",
            self.target.package, self.target.kind, self.target.name, self.name
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Killed,
    Ignored,
    Timeout,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationStatus {
    Tested,
    Unviable,
    BuildTimeout,
    Inconclusive,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MutationResult {
    pub mutant: Mutant,
    pub status: MutationStatus,
    // Entries have the same order as Analysis.tests.
    pub outcomes: Vec<Outcome>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Analysis {
    pub schema_version: u32,
    pub workspace: String,
    pub package: Option<String>,
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
    pub generated_mutants: usize,
    pub tests: Vec<Test>,
    pub mutants: Vec<MutationResult>,
}

impl Analysis {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported report schema version"
        );
        let ids: BTreeSet<_> = self.tests.iter().map(Test::id).collect();
        ensure!(
            ids.len() == self.tests.len(),
            "duplicate test identities in report"
        );
        ensure!(
            self.generated_mutants >= self.mutants.len(),
            "invalid mutant count"
        );
        for test in &self.tests {
            ensure!(
                matches!(test.baseline, Outcome::Passed | Outcome::Ignored),
                "invalid baseline for {}",
                test.id()
            );
        }
        for result in &self.mutants {
            let executed = matches!(
                result.status,
                MutationStatus::Tested | MutationStatus::Inconclusive
            );
            ensure!(
                result.outcomes.len() == if executed { self.tests.len() } else { 0 },
                "invalid matrix dimensions"
            );
            if result.status == MutationStatus::Tested {
                ensure!(
                    result
                        .outcomes
                        .iter()
                        .all(|o| matches!(o, Outcome::Passed | Outcome::Killed | Outcome::Ignored)),
                    "uncertain outcomes in a completed mutant"
                );
            }
        }
        Ok(())
    }

    pub fn kills(&self) -> Vec<BTreeSet<usize>> {
        (0..self.tests.len())
            .map(|test| {
                self.mutants
                    .iter()
                    .enumerate()
                    .filter_map(|(index, result)| {
                        (result.status == MutationStatus::Tested
                            && result.outcomes[test] == Outcome::Killed)
                            .then_some(index)
                    })
                    .collect()
            })
            .collect()
    }
}
