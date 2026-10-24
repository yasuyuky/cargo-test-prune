use std::path::Path;

use crate::mutants::{Mutant, Position, Span};

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
