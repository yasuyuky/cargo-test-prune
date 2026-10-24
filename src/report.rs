use std::{io::Write, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    matrix::{Analysis, MutationStatus},
    minimize::Recommendation,
};

#[derive(Debug, Deserialize, Serialize)]
pub struct Report {
    pub analysis: Analysis,
    pub recommendation: Recommendation,
}

impl Report {
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        writeln!(file)?;
        file.persist(path)
            .with_context(|| format!("save {}", path.display()))?;
        Ok(())
    }

    pub fn print(&self) {
        let analysis = &self.analysis;
        let recommendation = &self.recommendation;
        let completed = analysis
            .mutants
            .iter()
            .filter(|m| m.status == MutationStatus::Tested)
            .count();
        println!(
            "Analyzed: {} tests, {}/{} generated mutants",
            analysis.tests.len(),
            analysis.mutants.len(),
            analysis.generated_mutants
        );
        println!(
            "Evidence: {} fully observed, {} detected, {} survived, {} excluded",
            completed,
            recommendation.detected_mutants,
            completed - recommendation.detected_mutants,
            analysis.mutants.len() - completed
        );
        println!("\nRemoval candidates (one combined proposal):");
        if recommendation.candidates.is_empty() {
            println!("  None");
        }
        for candidate in &recommendation.candidates {
            println!("  {}", candidate.test);
            println!(
                "    {0}/{0} observed kills covered; no unique kills",
                candidate.killed_mutants.len()
            );
            println!(
                "    Retained witnesses: {}",
                candidate.covered_by.join(", ")
            );
        }
        println!("\nRetain:");
        for retained in &recommendation.retained {
            println!("  {} — {}", retained.test, retained.reason);
            if !retained.unique_mutants.is_empty() {
                println!("    Unique mutants: {:?}", retained.unique_mutants);
            }
        }
        println!(
            "\nSuggested removal: {} tests; retain {} tests",
            recommendation.candidates.len(),
            recommendation.retained.len()
        );
        println!(
            "Confidence: limited to observed mutants. Review specification value before deletion."
        );
        println!("No source files were changed. Greedy selection is not a guaranteed minimum.");
    }
}
