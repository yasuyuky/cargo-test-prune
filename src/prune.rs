use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use syn::{Item, ext::IdentExt, spanned::Spanned};

use crate::{matrix::Test, report::Report, runner, runner::Runner};

struct Edit {
    path: PathBuf,
    before: String,
    after: String,
}

pub fn apply(runner: &Runner<'_>, report: &Report) -> Result<()> {
    let candidates: BTreeSet<_> = report
        .recommendation
        .candidates
        .iter()
        .map(|decision| decision.test.as_str())
        .collect();
    if candidates.is_empty() {
        println!("No removal candidates; no source files were changed.");
        return Ok(());
    }
    let mut removals: BTreeMap<PathBuf, Vec<Range<usize>>> = BTreeMap::new();
    for test in &report.analysis.tests {
        if !candidates.contains(test.id().as_str()) {
            continue;
        }
        let source = target_source(runner, test)?;
        let (path, range) = locate(
            &source,
            source.parent().context("target source has no parent")?,
            &test.name.split("::").collect::<Vec<_>>(),
        )
        .with_context(|| format!("cannot safely remove {}; retain it with --keep", test.id()))?;
        ensure!(
            path.starts_with(&runner.workspace.root),
            "test source is outside the analysis copy: {}",
            path.display()
        );
        removals.entry(path).or_default().push(range);
    }
    let mut edits = Vec::new();
    for (path, mut ranges) in removals {
        ranges.sort_by_key(|range| range.start);
        ranges.dedup();
        ensure!(
            ranges.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "overlapping test source ranges: {}",
            path.display()
        );
        let before = fs::read_to_string(&path)?;
        let mut after = before.clone();
        for range in ranges.into_iter().rev() {
            after.replace_range(range, "");
        }
        edits.push(Edit {
            path: path.strip_prefix(&runner.workspace.root)?.to_owned(),
            before,
            after,
        });
    }
    for edit in &edits {
        fs::write(runner.workspace.root.join(&edit.path), &edit.after)?;
    }
    eprintln!("Verifying removal in the workspace copy...");
    let build = runner.build()?;
    ensure!(
        build.success(),
        "removal verification build failed; original sources are unchanged:\n{}",
        build.diagnostic()
    );
    let remaining = runner.discover(&build)?;
    let expected: BTreeSet<_> = report
        .recommendation
        .retained
        .iter()
        .map(|decision| decision.test.clone())
        .collect();
    ensure!(
        remaining.iter().map(Test::id).collect::<BTreeSet<_>>() == expected,
        "removal changed unexpected test identities; original sources are unchanged"
    );
    for test in &report.analysis.tests {
        if !expected.contains(&test.id()) {
            continue;
        }
        let output = runner.test(test)?;
        ensure!(
            runner::outcome(&output) == test.baseline,
            "retained test failed verification: {}; original sources are unchanged:\n{}",
            test.id(),
            output.diagnostic()
        );
    }
    commit(&runner.workspace.original, &edits)?;
    println!(
        "Removed {} tests from {} source files; remaining tests verified.",
        candidates.len(),
        edits.len()
    );
    Ok(())
}

fn target_source(runner: &Runner<'_>, test: &Test) -> Result<PathBuf> {
    let packages = runner.workspace.metadata["packages"]
        .as_array()
        .context("missing packages")?;
    let package = packages
        .iter()
        .find(|package| package["name"] == test.target.package)
        .context("missing test package")?;
    let target = package["targets"]
        .as_array()
        .context("missing targets")?
        .iter()
        .find(|target| {
            target["name"] == test.target.name
                && target["kind"].as_array().is_some_and(|kinds| {
                    kinds.iter().any(|kind| {
                        kind == &test.target.kind
                            || (test.target.kind == "lib"
                                && matches!(
                                    kind.as_str(),
                                    Some("rlib" | "dylib" | "staticlib" | "cdylib" | "proc-macro")
                                ))
                    })
                })
        })
        .context("missing test target")?;
    Ok(PathBuf::from(
        target["src_path"]
            .as_str()
            .context("missing target source")?,
    ))
}

fn locate(path: &Path, directory: &Path, names: &[&str]) -> Result<(PathBuf, Range<usize>)> {
    let source = fs::read_to_string(path)?;
    let file = syn::parse_file(&source)
        .with_context(|| format!("parse test source {}", path.display()))?;
    // parse_file strips these prefixes before producing byte spans.
    let offset = usize::from(source.starts_with('\u{feff}')) * '\u{feff}'.len_utf8()
        + file.shebang.as_ref().map_or(0, String::len);
    locate_items(path, directory, &source, offset, &file.items, names)
}

fn locate_items(
    path: &Path,
    directory: &Path,
    source: &str,
    offset: usize,
    items: &[Item],
    names: &[&str],
) -> Result<(PathBuf, Range<usize>)> {
    let (name, rest) = names.split_first().context("empty test name")?;
    let matches: Vec<_> = items
        .iter()
        .filter(|item| match item {
            Item::Fn(function) if rest.is_empty() => function.sig.ident.unraw() == *name,
            Item::Mod(module) if !rest.is_empty() => module.ident.unraw() == *name,
            _ => false,
        })
        .collect();
    ensure!(
        matches.len() == 1,
        "expected one source definition for {name} in {}; macro-generated or ambiguous tests are unsupported",
        path.display()
    );
    match matches[0] {
        Item::Fn(function) => {
            ensure!(
                function
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("test")),
                "only explicit #[test] functions can be removed"
            );
            let span = function.span().byte_range();
            let mut range = span.start + offset..span.end + offset;
            let line_start = source[..range.start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = source[range.end..]
                .find('\n')
                .map_or(source.len(), |i| range.end + i + 1);
            if source[line_start..range.start].trim().is_empty()
                && source[range.end..line_end].trim().is_empty()
            {
                range = line_start..line_end;
            }
            Ok((path.to_owned(), range))
        }
        Item::Mod(module) => {
            ensure!(
                !module.attrs.iter().any(|attr| {
                    attr.path().is_ident("path") || attr.path().is_ident("cfg_attr")
                }),
                "modules with #[path] or #[cfg_attr] are unsupported for removal"
            );
            let directory = directory.join(name);
            if let Some((_, items)) = &module.content {
                return locate_items(path, &directory, source, offset, items, rest);
            }
            let paths: Vec<_> = [directory.with_extension("rs"), directory.join("mod.rs")]
                .into_iter()
                .filter(|path| path.is_file())
                .collect();
            ensure!(
                paths.len() == 1,
                "cannot resolve module {name} unambiguously"
            );
            locate(&paths[0], &directory, rest)
        }
        _ => unreachable!(),
    }
}

fn commit(root: &Path, edits: &[Edit]) -> Result<()> {
    let mut staged = Vec::new();
    for edit in edits {
        let path = root.join(&edit.path);
        // Check every file before replacing any, including newly introduced symlinks.
        let mut component = root.to_owned();
        for part in edit.path.components() {
            component.push(part);
            ensure!(
                !fs::symlink_metadata(&component)?.file_type().is_symlink(),
                "source path became a symlink: {}",
                component.display()
            );
        }
        ensure!(
            fs::read_to_string(&path)? == edit.before,
            "source changed since analysis: {}; no removals applied",
            path.display()
        );
        let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        file.as_file()
            .set_permissions(fs::metadata(&path)?.permissions())?;
        file.write_all(edit.after.as_bytes())?;
        staged.push((path, file));
    }
    for (index, (path, file)) in staged.into_iter().enumerate() {
        if let Err(error) = file.persist(&path) {
            for previous in &edits[..index] {
                fs::write(root.join(&previous.path), &previous.before)
                    .context("failed to restore sources after a write error")?;
            }
            return Err(error).with_context(|| format!("replace {}", path.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remove(source: &str, name: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        fs::write(&path, source).unwrap();
        let (_, range) = locate(&path, dir.path(), &name.split("::").collect::<Vec<_>>()).unwrap();
        let mut result = source.to_owned();
        result.replace_range(range, "");
        result
    }

    #[test]
    fn removes_attributes_and_docs_without_touching_neighbors() {
        let before = "mod tests {\n    // Keep this context.\n    /// 冗長なテスト\n    #[test]\n    #[should_panic]\n    fn r#type() { panic!(\"} /* { */\"); }\n    fn helper() {}\n}\n";
        assert_eq!(
            remove(before, "tests::type"),
            "mod tests {\n    // Keep this context.\n    fn helper() {}\n}\n"
        );
        assert_eq!(
            remove(
                "fn helper() {} #[test] fn candidate() {} fn kept() {}",
                "candidate"
            ),
            "fn helper() {}  fn kept() {}"
        );
    }
}
