use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::matrix::{Analysis, Outcome};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    #[default]
    Count,
    Time,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Decision {
    pub test: String,
    pub reason: String,
    pub killed_mutants: Vec<usize>,
    pub unique_mutants: Vec<usize>,
    pub covered_by: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Recommendation {
    pub weight: Weight,
    pub detected_mutants: usize,
    pub retained: Vec<Decision>,
    pub candidates: Vec<Decision>,
}

pub fn minimize(analysis: &Analysis, weight: Weight, keep: &[String]) -> Result<Recommendation> {
    analysis.validate()?;
    let ids: Vec<_> = analysis.tests.iter().map(|test| test.id()).collect();
    for pattern in keep {
        ensure!(
            ids.iter().any(|id| id.contains(pattern)),
            "--keep pattern matched no tests: {pattern}"
        );
    }
    let kills = analysis.kills();
    let costs: Vec<_> = analysis
        .tests
        .iter()
        .map(|test| match weight {
            Weight::Count => 1,
            Weight::Time => u128::from(test.duration_ms.max(1)),
        })
        .collect();
    let universe: BTreeSet<_> = kills.iter().flatten().copied().collect();
    let reasons: Vec<_> = analysis
        .tests
        .iter()
        .enumerate()
        .map(|(i, test)| {
            if keep.iter().any(|pattern| ids[i].contains(pattern)) {
                Some("explicitly kept")
            } else if test.baseline == Outcome::Ignored {
                Some("ignored; no baseline observation")
            } else if analysis.mutants.iter().any(|result| {
                result.outcomes.get(i).is_some_and(|o| {
                    matches!(o, Outcome::Timeout | Outcome::Unknown | Outcome::Ignored)
                })
            }) {
                Some("inconclusive observations")
            } else if kills[i].is_empty() {
                Some("no observed kills; insufficient evidence")
            } else {
                None
            }
        })
        .collect();
    let mut retained: BTreeSet<_> = (0..ids.len()).filter(|&i| reasons[i].is_some()).collect();
    let mut covered: BTreeSet<_> = retained.iter().flat_map(|&i| &kills[i]).copied().collect();
    while covered != universe {
        let mut best = None;
        let mut best_gain = 0_u128;
        let mut best_cost = 1_u128;
        for (i, &cost) in costs.iter().enumerate() {
            if retained.contains(&i) {
                continue;
            }
            let gain = kills[i].difference(&covered).count() as u128;
            if gain * best_cost > best_gain * cost {
                best = Some(i);
                best_gain = gain;
                best_cost = cost;
            }
        }
        let i = best.expect("every observed kill has a covering test");
        retained.insert(i);
        covered.extend(&kills[i]);
    }
    loop {
        // Greedy choices and swaps can make other selections redundant.
        for i in retained.clone().into_iter().rev() {
            if reasons[i].is_some() {
                continue;
            }
            let others: BTreeSet<_> = retained
                .iter()
                .filter(|&&j| j != i)
                .flat_map(|&j| &kills[j])
                .copied()
                .collect();
            if kills[i].is_subset(&others) {
                retained.remove(&i);
            }
        }
        let removable: Vec<_> = retained
            .iter()
            .copied()
            .filter(|&i| reasons[i].is_none())
            .collect();
        let Some((a, b, replacement)) = find_swap(&kills, &costs, &retained, &removable) else {
            break;
        };
        retained.remove(&a);
        retained.remove(&b);
        retained.insert(replacement);
    }
    let mut recommendation = Recommendation {
        weight,
        detected_mutants: universe.len(),
        retained: Vec::new(),
        candidates: Vec::new(),
    };
    for i in 0..ids.len() {
        let unique: Vec<_> = kills[i]
            .iter()
            .filter(|m| kills.iter().filter(|set| set.contains(m)).count() == 1)
            .map(|m| m + 1)
            .collect();
        let decision = Decision {
            test: ids[i].clone(),
            reason: reasons[i]
                .unwrap_or(if !unique.is_empty() {
                    "uniquely detects mutants"
                } else if retained.contains(&i) {
                    "selected to preserve observed detection"
                } else {
                    "all observed kills covered by retained tests"
                })
                .to_owned(),
            killed_mutants: kills[i].iter().map(|m| m + 1).collect(),
            unique_mutants: unique,
            covered_by: if retained.contains(&i) {
                Vec::new()
            } else {
                retained
                    .iter()
                    .filter(|&&j| !kills[i].is_disjoint(&kills[j]))
                    .map(|&j| ids[j].clone())
                    .collect()
            },
        };
        if retained.contains(&i) {
            recommendation.retained.push(decision);
        } else {
            recommendation.candidates.push(decision);
        }
    }
    Ok(recommendation)
}

fn find_swap(
    kills: &[BTreeSet<usize>],
    costs: &[u128],
    retained: &BTreeSet<usize>,
    removable: &[usize],
) -> Option<(usize, usize, usize)> {
    for (index, &a) in removable.iter().enumerate() {
        for &b in &removable[index + 1..] {
            let others: BTreeSet<_> = retained
                .iter()
                .filter(|&&i| i != a && i != b)
                .flat_map(|&i| &kills[i])
                .copied()
                .collect();
            let needed: BTreeSet<_> = kills[a]
                .union(&kills[b])
                .copied()
                .filter(|m| !others.contains(m))
                .collect();
            if let Some(replacement) = (0..kills.len()).find(|&i| {
                !retained.contains(&i)
                    && costs[i] < costs[a] + costs[b]
                    && needed.is_subset(&kills[i])
            }) {
                return Some((a, b, replacement));
            }
        }
    }
    None
}
