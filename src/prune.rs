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
