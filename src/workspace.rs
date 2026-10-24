use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tempfile::TempDir;

use crate::process;

pub fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.env("CARGO_TERM_COLOR", "never");
    command
}

pub struct Workspace {
    _temp: TempDir,
    pub root: PathBuf,
    pub original: PathBuf,
    pub metadata: Value,
}

impl Workspace {
    pub fn copy(manifest: Option<&Path>, timeout: Duration) -> Result<Self> {
        let mut command = cargo();
        command.args(["metadata", "--no-deps", "--format-version=1"]);
        if let Some(manifest) = manifest {
            command.arg("--manifest-path").arg(manifest);
        }
        let output = process::run(&mut command, timeout)?;
        ensure!(
            output.success(),
            "cargo metadata failed or timed out:\n{}",
            output.diagnostic()
        );
        let metadata: Value = serde_json::from_str(&output.stdout)?;
        let original = PathBuf::from(
            metadata["workspace_root"]
                .as_str()
                .context("missing workspace root")?,
        )
        .canonicalize()?;
        for package in metadata["packages"]
            .as_array()
            .context("missing packages")?
        {
            let manifest = Path::new(
                package["manifest_path"]
                    .as_str()
                    .context("missing manifest path")?,
            );
            ensure!(
                manifest.starts_with(&original),
                "workspace member outside workspace is unsupported"
            );
            check_manifest(manifest)?;
            for target in package["targets"].as_array().context("missing targets")? {
                let source = Path::new(
                    target["src_path"]
                        .as_str()
                        .context("missing target source")?,
                );
                ensure!(
                    source.canonicalize()?.starts_with(&original),
                    "target source outside workspace is unsupported: {}",
                    source.display()
                );
            }
            for dependency in package["dependencies"]
                .as_array()
                .context("missing dependencies")?
            {
                if let Some(path) = dependency["path"].as_str() {
                    ensure!(
                        Path::new(path).canonicalize()?.starts_with(&original),
                        "external path dependency is unsupported: {path}; put it inside the workspace"
                    );
                }
            }
        }
        check_manifest(&original.join("Cargo.toml"))?;
        let temp = tempfile::tempdir().context("create analysis workspace")?;
        let root = temp.path().join("workspace");
        let target = PathBuf::from(
            metadata["target_directory"]
                .as_str()
                .context("missing target directory")?,
        );
        copy_tree(&original, &root, &target)?;
        let root = root.canonicalize()?;
        let output = process::run(
            cargo()
                .current_dir(&root)
                .args(["metadata", "--no-deps", "--format-version=1"]),
            timeout,
        )?;
        ensure!(
            output.success(),
            "metadata failed in workspace copy:\n{}",
            output.diagnostic()
        );
        let metadata = serde_json::from_str(&output.stdout)?;
        Ok(Self {
            _temp: temp,
            root,
            original,
            metadata,
        })
    }
}

fn check_manifest(path: &Path) -> Result<()> {
    let value: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
    fn check(value: &toml::Value) -> Result<()> {
        match value {
            toml::Value::Table(table) => {
                for (key, value) in table {
                    if matches!(key.as_str(), "path" | "build")
                        && let Some(path) = value.as_str()
                    {
                        ensure!(
                            !Path::new(path).is_absolute(),
                            "absolute manifest paths are unsupported: {path}; use workspace-relative paths"
                        );
                    }
                    check(value)?;
                }
            }
            toml::Value::Array(values) => {
                for value in values {
                    check(value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    check(&value).with_context(|| format!("read {}", path.display()))
}

fn copy_tree(source: &Path, destination: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        if matches!(entry.file_name().to_str(), Some(".git" | "target")) || path == target {
            continue;
        }
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "symlinks are unsupported in the analysis copy: {}",
            path.display()
        );
        let to = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&path, &to, target)?;
        } else if kind.is_file() {
            fs::copy(&path, &to)?;
        } else {
            anyhow::bail!("unsupported file type: {}", path.display());
        }
    }
    Ok(())
}
