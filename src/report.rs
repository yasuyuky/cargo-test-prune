use std::{
    io::{BufWriter, Write},
    path::Path,
};

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
        let mut writer = BufWriter::new(tempfile::NamedTempFile::new_in(parent)?);
        serde_json::to_writer_pretty(&mut writer, self)?;
        writeln!(writer)?;
        writer
            .into_inner()?
            .persist(path)
            .with_context(|| format!("save {}", path.display()))?;
        Ok(())
    }

    pub fn print_mermaid(&self) {
        println!("# Test removal candidates\n");
        println!("These diagrams describe one combined proposal, limited to observed mutants.");
        println!("Solid arrows: retained tests. Dotted arrows: removal candidate.");
        println!("Mutant numbers are one-based indices into analysis.mutants.");
        println!(
            "Review specification value before deletion; the selection is not a guaranteed minimum.\n"
        );
        if self.recommendation.candidates.is_empty() {
            println!("No removal candidates.");
        }
        for (index, candidate) in self.recommendation.candidates.iter().enumerate() {
            let witnesses: Vec<_> = self
                .recommendation
                .retained
                .iter()
                .enumerate()
                .filter(|(_, retained)| candidate.covered_by.contains(&retained.test))
                .collect();
            println!("## Candidate {}\n", index + 1);
            println!("```mermaid\nflowchart LR");
            println!("    subgraph tests[\"Tests\"]");
            println!(
                "        candidate[\"Candidate: {}\"]:::candidate",
                mermaid_label(&candidate.test)
            );
            for (i, retained) in &witnesses {
                println!(
                    "        retained_{i}[\"Retain: {}\"]:::retained",
                    mermaid_label(&retained.test)
                );
            }
            println!("    end\n    subgraph mutants[\"Detected mutants\"]");
            for number in &candidate.killed_mutants {
                let mutant = &self.analysis.mutants[number - 1].mutant;
                println!(
                    "        mutant_{number}[\"M{number}: {}\"]",
                    mermaid_label(&mutant.name)
                );
            }
            println!("    end");
            for number in &candidate.killed_mutants {
                println!("    candidate -.-> mutant_{number}");
                for (i, retained) in &witnesses {
                    if retained.killed_mutants.contains(number) {
                        println!("    retained_{i} --> mutant_{number}");
                    }
                }
            }
            println!("    classDef retained fill:#dcfce7,stroke:#15803d,color:#14532d");
            println!("    classDef candidate fill:#fef3c7,stroke:#b45309,color:#78350f");
            println!("```\n");
        }
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
        println!("Greedy selection is not a guaranteed minimum.");
    }
}

fn mermaid_label(value: &str) -> String {
    let mut label = String::new();
    for ch in value.chars() {
        match ch {
            '"' | '&' | '#' | '<' | '>' | '`' | '\\' => {
                label.push_str(&format!("#{};", ch as u32));
            }
            ch if ch.is_whitespace() || ch.is_control() => label.push(' '),
            ch => label.push(ch),
        }
    }
    label
}
