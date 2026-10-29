use crate::{
    matrix::{Analysis, MutationResult, MutationStatus, Outcome, Target, Test},
    minimize::{Weight, minimize},
    mutants::{Mutant, Position, Span},
};

fn analysis(rows: &[&[usize]]) -> Analysis {
    let count = rows
        .iter()
        .flat_map(|row| row.iter())
        .max()
        .map_or(0, |m| m + 1);
    Analysis {
        schema_version: 1,
        workspace: ".".into(),
        package: None,
        features: vec![],
        all_features: false,
        no_default_features: false,
        generated_mutants: count,
        tests: rows
            .iter()
            .enumerate()
            .map(|(i, _)| Test {
                target: Target {
                    package: "demo".into(),
                    kind: "lib".into(),
                    name: "demo".into(),
                },
                name: format!("t{i}"),
                baseline: Outcome::Passed,
                duration_ms: 1,
            })
            .collect(),
        mutants: (0..count)
            .map(|i| MutationResult {
                mutant: Mutant {
                    name: format!("m{i}"),
                    file: "src/lib.rs".into(),
                    span: Span {
                        start: Position { line: 1, column: 1 },
                        end: Position { line: 1, column: 2 },
                    },
                    replacement: "0".into(),
                },
                status: MutationStatus::Tested,
                outcomes: rows
                    .iter()
                    .map(|row| {
                        if row.contains(&i) {
                            Outcome::Killed
                        } else {
                            Outcome::Passed
                        }
                    })
                    .collect(),
                diagnostic: None,
            })
            .collect(),
    }
}

#[test]
fn duplicate_tests_are_not_both_removed_and_unique_kills_are_retained() {
    let evidence = analysis(&[&[0, 1], &[0, 1], &[2]]);
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].test, evidence.tests[1].id());
    assert_eq!(
        result.candidates[0].covered_by,
        vec![evidence.tests[0].id()]
    );
    assert_eq!(result.retained[1].unique_mutants, vec![3]);
}

#[test]
fn unique_kills_use_all_tests_and_preserve_mutant_numbers() {
    let mut evidence = analysis(&[&[1, 4], &[1], &[4], &[6]]);
    evidence.mutants[2].status = MutationStatus::Unviable;
    evidence.mutants[2].outcomes.clear();
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(result.detected_mutants, 3);
    assert_eq!(result.retained.len(), 2);
    assert_eq!(result.retained[0].test, evidence.tests[0].id());
    assert_eq!(result.retained[0].killed_mutants, vec![2, 5]);
    assert!(result.retained[0].unique_mutants.is_empty());
    assert_eq!(result.retained[1].test, evidence.tests[3].id());
    assert_eq!(result.retained[1].unique_mutants, vec![7]);
}

#[test]
fn redundant_selections_do_not_remove_the_last_detector() {
    let mut evidence = analysis(&[&[0, 1], &[0, 2], &[1, 3], &[2, 4]]);
    evidence.tests[2].duration_ms = 10;
    evidence.tests[3].duration_ms = 10;
    let result = minimize(&evidence, Weight::Time, &[]).unwrap();
    assert_eq!(
        result
            .retained
            .iter()
            .map(|d| d.test.clone())
            .collect::<Vec<_>>(),
        [0, 2, 3].map(|i| evidence.tests[i].id())
    );
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].test, evidence.tests[1].id());
    assert_eq!(
        result.candidates[0].covered_by,
        [0, 3].map(|i| evidence.tests[i].id())
    );
}

#[test]
fn weighted_selection_and_explicit_pins_change_the_proposal() {
    let mut evidence = analysis(&[&[0, 1], &[0], &[1]]);
    evidence.tests[0].duration_ms = 100;
    assert_eq!(
        minimize(&evidence, Weight::Count, &[])
            .unwrap()
            .retained
            .len(),
        1
    );
    let result = minimize(&evidence, Weight::Time, &[]).unwrap();
    assert_eq!(result.candidates[0].test, evidence.tests[0].id());
    let pinned = minimize(&evidence, Weight::Time, &["/t0".into()]).unwrap();
    assert_eq!(pinned.retained.len(), 1);
    assert!(minimize(&evidence, Weight::Count, &["missing".into()]).is_err());
}

#[test]
fn lack_of_evidence_and_incomplete_runs_do_not_justify_removal() {
    let mut evidence = analysis(&[&[0], &[0], &[]]);
    evidence.mutants[0].status = MutationStatus::Inconclusive;
    evidence.mutants[0].outcomes[0] = Outcome::Timeout;
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(result.detected_mutants, 0);
    assert!(result.candidates.is_empty());
    evidence.mutants[0].status = MutationStatus::Tested;
    assert!(minimize(&evidence, Weight::Count, &[]).is_err());
}

fn swap_example() -> Analysis {
    analysis(&[&[1, 2, 4, 6], &[1, 3, 5, 6], &[0, 1, 4, 5], &[0, 2, 3, 4]])
}

#[test]
fn local_search_replaces_two_tests_and_preserves_detection() {
    let evidence = swap_example();
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    let expected = [1, 3].map(|i| evidence.tests[i].id());
    assert_eq!(
        result.retained.iter().map(|d| &d.test).collect::<Vec<_>>(),
        expected.iter().collect::<Vec<_>>()
    );
    assert_eq!(result.detected_mutants, 7);
    assert_eq!(result.candidates.len(), 2);
    for candidate in &result.candidates {
        assert_eq!(candidate.covered_by, expected);
        assert!(candidate.killed_mutants.iter().all(|m| {
            result
                .retained
                .iter()
                .any(|test| test.killed_mutants.contains(m))
        }));
    }
}

#[test]
fn local_search_repeats_for_independent_groups() {
    let evidence = analysis(&[
        &[1, 2, 4, 6],
        &[1, 3, 5, 6],
        &[0, 1, 4, 5],
        &[0, 2, 3, 4],
        &[8, 9, 11, 13],
        &[8, 10, 12, 13],
        &[7, 8, 11, 12],
        &[7, 9, 10, 11],
    ]);
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(
        result
            .retained
            .iter()
            .map(|d| d.test.clone())
            .collect::<Vec<_>>(),
        [1, 3, 5, 7].map(|i| evidence.tests[i].id())
    );
}

#[test]
fn local_search_removes_selections_made_redundant_by_a_swap() {
    let evidence = analysis(&[
        &[1, 3, 4, 5],
        &[0, 3, 6],
        &[5, 7],
        &[0, 2, 4, 5],
        &[2, 3, 5, 6],
        &[1, 3, 6, 7],
        &[1, 3, 5],
    ]);
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(
        result
            .retained
            .iter()
            .map(|d| d.test.clone())
            .collect::<Vec<_>>(),
        [3, 5].map(|i| evidence.tests[i].id())
    );
}

#[test]
fn local_search_only_accepts_strictly_cheaper_time_costs() {
    for (cost, replacement_cost, swapped) in [
        (10, 19, true),
        (10, 20, false),
        (10, 21, false),
        (u64::MAX - 1, u64::MAX, true),
    ] {
        let mut evidence = swap_example();
        for test in &mut evidence.tests {
            test.duration_ms = cost;
        }
        evidence.tests[3].duration_ms = replacement_cost;
        let result = minimize(&evidence, Weight::Time, &[]).unwrap();
        let expected = if swapped { vec![1, 3] } else { vec![0, 1, 2] };
        assert_eq!(
            result
                .retained
                .iter()
                .map(|d| d.test.clone())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|&i| evidence.tests[i].id())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn local_search_preserves_pins_and_inconclusive_tests() {
    let mut evidence = swap_example();
    let pinned = minimize(&evidence, Weight::Count, &["/t0".into()]).unwrap();
    assert_eq!(pinned.retained.len(), 3);
    assert_eq!(pinned.retained[0].test, evidence.tests[0].id());
    assert_eq!(pinned.retained[0].reason, "explicitly kept");

    let mut incomplete = analysis(&[&[0], &[], &[], &[]]).mutants.remove(0);
    incomplete.status = MutationStatus::Inconclusive;
    incomplete.outcomes[0] = Outcome::Timeout;
    evidence.mutants.push(incomplete);
    evidence.generated_mutants += 1;
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(result.retained.len(), 3);
    assert_eq!(result.retained[0].test, evidence.tests[0].id());
    assert_eq!(result.retained[0].reason, "inconclusive observations");
}

#[test]
fn local_search_keeps_ignored_tests_and_tests_without_kills() {
    let mut evidence = analysis(&[&[0], &[0], &[], &[]]);
    evidence.tests[2].baseline = Outcome::Ignored;
    evidence.mutants[0].outcomes[2] = Outcome::Ignored;
    let result = minimize(&evidence, Weight::Count, &[]).unwrap();
    assert_eq!(
        result
            .retained
            .iter()
            .map(|d| d.test.clone())
            .collect::<Vec<_>>(),
        [0, 2, 3].map(|i| evidence.tests[i].id())
    );
}
