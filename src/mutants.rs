use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Mutant {
    pub name: String,
    pub file: PathBuf,
    pub span: Span,
    pub replacement: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl Mutant {
    pub fn path(&self, root: &Path) -> Result<PathBuf> {
        ensure!(
            !self.file.as_os_str().is_empty()
                && self
                    .file
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "mutant path must be workspace-relative: {}",
            self.file.display()
        );
        let path = root.join(&self.file).canonicalize()?;
        ensure!(path.starts_with(root), "mutant path escapes the workspace");
        ensure!(
            path.extension().is_some_and(|ext| ext == "rs"),
            "mutant must target a Rust file"
        );
        Ok(path)
    }

    pub fn apply(&self, source: &str) -> Result<String> {
        let start = offset(source, self.span.start)?;
        let end = offset(source, self.span.end)?;
        ensure!(start <= end, "reversed mutation span for {}", self.name);
        let mut changed = source.to_owned();
        changed.replace_range(start..end, &self.replacement);
        Ok(changed)
    }
}

fn offset(source: &str, position: Position) -> Result<usize> {
    let mut line = 1;
    let mut column = 1;
    // cargo-mutants uses one-based character columns, excluding CR in CRLF.
    for (offset, ch) in source.char_indices().chain([(source.len(), '\0')]) {
        if line == position.line && column == position.column {
            return Ok(offset);
        }
        match ch {
            '\n' => {
                line += 1;
                column = 1;
            }
            '\r' => {}
            _ => column += 1,
        }
    }
    None.context(format!(
        "mutation position {}:{} is outside the source",
        position.line, position.column
    ))
}
