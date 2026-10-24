use std::path::PathBuf;

use clap::Args;

use crate::minimize::Weight;

#[derive(Args)]
struct Selection {
    /// Minimize test count or measured baseline command time
    #[arg(long, value_enum, default_value = "count")]
    weight: Weight,
    /// Retain every test whose full identity contains this string (repeatable)
    #[arg(long)]
    keep: Vec<String>,
}

#[derive(Args)]
struct Analyze {
    #[arg(long)]
    manifest_path: Option<PathBuf>,
    /// Analyze only this package; defaults to the entire workspace
    #[arg(short, long)]
    package: Option<String>,
    #[arg(long, value_delimiter = ',')]
    features: Vec<String>,
    #[arg(long)]
    all_features: bool,
    #[arg(long)]
    no_default_features: bool,
    /// Analyze at most this many mutants in generation order
    #[arg(long, default_value = "50", value_parser = clap::value_parser!(u32).range(1..))]
    max_mutants: u32,
    /// Per-test cargo command timeout, in seconds
    #[arg(long, default_value = "30", value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,
    /// Per-build/discovery timeout, in seconds
    #[arg(long, default_value = "300", value_parser = clap::value_parser!(u64).range(1..))]
    build_timeout: u64,
    /// Read a fresh `cargo mutants --list --json` file instead of generating mutants
    #[arg(long)]
    mutants_file: Option<PathBuf>,
    /// Save all observations and the proposal as JSON
    #[arg(short, long, default_value = "test-prune.json")]
    output: PathBuf,
    #[command(flatten)]
    selection: Selection,
}
