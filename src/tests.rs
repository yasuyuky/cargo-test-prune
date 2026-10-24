use std::{collections::BTreeSet, path::Path};

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
fn every_small_matrix_preserves_the_union_when_candidates_are_removed_together() {
    for bits in 0_u16..512 {
        let rows: Vec<Vec<_>> = (0..3)
            .map(|t| (0..3).filter(|m| bits & (1 << (t * 3 + m)) != 0).collect())
            .collect();
        let evidence = analysis(&rows.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let result = minimize(&evidence, Weight::Count, &[]).unwrap();
        let original: BTreeSet<_> = rows.iter().flatten().copied().collect();
        let remaining: BTreeSet<_> = result
            .retained
            .iter()
            .flat_map(|t| t.killed_mutants.iter().map(|m| m - 1))
            .collect();
        assert_eq!(remaining, original, "matrix {bits}");
        for candidate in result.candidates {
            assert!(!candidate.killed_mutants.is_empty());
            assert!(candidate.unique_mutants.is_empty());
        }
    }
}

#[test]
fn applies_character_spans_without_changing_surrounding_unicode_or_crlf() {
    let mutant = Mutant {
        name: "replace".into(),
        file: "src/lib.rs".into(),
        replacement: "false".into(),
        span: Span {
            start: Position { line: 2, column: 5 },
            end: Position { line: 3, column: 4 },
        },
    };
    assert_eq!(
        mutant
            .apply("// 日本語\r\n    日本\r\n語です\r\nend")
            .unwrap(),
        "// 日本語\r\n    false\r\nend"
    );
    let mut invalid = mutant;
    invalid.span.start.line = 99;
    assert!(invalid.apply("short").is_err());
    invalid.file = "../outside.rs".into();
    assert!(invalid.path(Path::new("/tmp")).is_err());
}
