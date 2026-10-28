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
