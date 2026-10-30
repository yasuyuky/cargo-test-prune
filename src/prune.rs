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
