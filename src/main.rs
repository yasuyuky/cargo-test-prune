mod matrix;
mod minimize;
mod mutants;
mod process;
mod report;
mod runner;
mod workspace;

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};

use matrix::{Analysis, MutationResult, MutationStatus, Outcome};
use minimize::{Weight, minimize};
use mutants::Mutant;
use report::Report;
use runner::Runner;
use workspace::Workspace;

#[derive(Parser)]
#[command(
    name = "cargo test-prune",
    version,
    about = "Find tests with overlapping mutation detection, without deleting source"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Measure per-test mutant detection in a temporary workspace copy
    Analyze(Analyze),
    /// Recompute a proposal from saved evidence, without running tests
    Report {
        input: PathBuf,
        #[command(flatten)]
        selection: Selection,
    },
}

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

fn main() -> Result<()> {
    process::handle_interrupts()?;
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "test-prune") {
        args.remove(1);
    }
    match Cli::parse_from(args).command {
        Action::Analyze(options) => analyze(options),
        Action::Report { input, selection } => {
            let mut report: Report = serde_json::from_str(&fs::read_to_string(input)?)?;
            report.recommendation = minimize(&report.analysis, selection.weight, &selection.keep)?;
            report.print();
            Ok(())
        }
    }
}

fn analyze(options: Analyze) -> Result<()> {
    eprintln!("Copying workspace...");
    let workspace = Workspace::copy(
        options.manifest_path.as_deref(),
        Duration::from_secs(options.build_timeout),
    )?;
    let runner = Runner {
        workspace: &workspace,
        options: &options,
    };
    eprintln!("Building baseline and discovering tests...");
    let build = runner.build()?;
    ensure!(
        build.success(),
        "baseline build failed or timed out:\n{}",
        build.diagnostic()
    );
    let mut tests = runner.discover(&build)?;
    let test_count = tests.len();
    for (index, test) in tests.iter_mut().enumerate() {
        eprintln!("Baseline {}/{}: {}", index + 1, test_count, test.id());
        let output = runner.test(test)?;
        test.baseline = runner::outcome(&output);
        test.duration_ms = output.elapsed_ms;
        ensure!(
            matches!(test.baseline, Outcome::Passed | Outcome::Ignored),
            "baseline test did not pass: {} ({:?})\n{}",
            test.id(),
            test.baseline,
            output.diagnostic()
        );
    }
    let mut analysis = Analysis {
        schema_version: 1,
        workspace: workspace.original.display().to_string(),
        package: options.package.clone(),
        features: options.features.clone(),
        all_features: options.all_features,
        no_default_features: options.no_default_features,
        generated_mutants: 0,
        tests,
        mutants: Vec::new(),
    };
    // Validate user pins before spending time on mutations.
    minimize(&analysis, options.selection.weight, &options.selection.keep)?;
    let mutations: Vec<Mutant> = if let Some(path) = &options.mutants_file {
        serde_json::from_str(&fs::read_to_string(path)?)?
    } else {
        let mut command = runner.command();
        command.args(["mutants", "--list", "--json", "--no-config"]);
        if let Some(package) = &options.package {
            command.args(["-p", package]);
        } else {
            command.arg("--workspace");
        }
        runner.features(&mut command);
        let output = process::run(&mut command, Duration::from_secs(options.build_timeout))?;
        ensure!(
            output.success(),
            "mutant generation failed; install cargo-mutants with `cargo install cargo-mutants --locked`\n{}",
            output.diagnostic()
        );
        serde_json::from_str(&output.stdout).context("parse cargo-mutants --list --json output")?
    };
    analysis.generated_mutants = mutations.len();
    let count = mutations.len().min(options.max_mutants as usize);
    for (index, mutant) in mutations.into_iter().take(count).enumerate() {
        eprintln!("Mutant {}/{}: {}", index + 1, count, mutant.name);
        let path = mutant.path(&workspace.root)?;
        let source = fs::read_to_string(&path)?;
        let changed = mutant.apply(&source)?;
        ensure!(
            source != changed,
            "mutation does not change source: {}",
            mutant.name
        );
        fs::write(&path, changed)?;
        let result = measure(&runner, mutant, &analysis.tests);
        fs::write(&path, source).context("restore analysis copy")?;
        analysis.mutants.push(result?);
    }
    let recommendation = minimize(&analysis, options.selection.weight, &options.selection.keep)?;
    let report = Report {
        analysis,
        recommendation,
    };
    report.save(&options.output)?;
    report.print();
    eprintln!("Evidence saved to {}", options.output.display());
    Ok(())
}

fn measure(runner: &Runner<'_>, mutant: Mutant, tests: &[matrix::Test]) -> Result<MutationResult> {
    let build = runner.build()?;
    if !build.success() {
        return Ok(MutationResult {
            mutant,
            status: if build.status.is_none() {
                MutationStatus::BuildTimeout
            } else {
                MutationStatus::Unviable
            },
            outcomes: Vec::new(),
            diagnostic: Some(build.diagnostic()),
        });
    }
    let mut outcomes = Vec::new();
    let mut diagnostics = Vec::new();
    for test in tests {
        if test.baseline == Outcome::Ignored {
            outcomes.push(Outcome::Ignored);
            continue;
        }
        let output = runner.test(test)?;
        let outcome = runner::outcome(&output);
        if matches!(
            outcome,
            Outcome::Unknown | Outcome::Timeout | Outcome::Ignored
        ) {
            diagnostics.push(format!(
                "{}: {outcome:?}\n{}",
                test.id(),
                output.diagnostic()
            ));
        }
        outcomes.push(outcome);
    }
    Ok(MutationResult {
        mutant,
        status: if diagnostics.is_empty() {
            MutationStatus::Tested
        } else {
            MutationStatus::Inconclusive
        },
        outcomes,
        diagnostic: (!diagnostics.is_empty()).then(|| diagnostics.join("\n")),
    })
}
