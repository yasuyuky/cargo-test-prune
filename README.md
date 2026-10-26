# cargo-test-prune

Find Rust tests whose **observed mutant detection** is covered by tests you keep.

AI-assisted development makes adding regression tests easy; deciding which old
tests still add value is harder. This tool measures which deliberate bugs each
test catches, then proposes a smaller test set with the same observed detection.
It reports evidence for review and never deletes tests.

## Try it

Use a current stable Rust toolchain and install
[cargo-mutants](https://mutants.rs/):

```sh
cargo install cargo-mutants --locked
cargo install cargo-test-prune --locked

# In the project to analyze:
cargo test-prune analyze
cargo test-prune analyze -p parser --max-mutants 100
cargo test-prune analyze --manifest-path ../parser/Cargo.toml --features serde
```

The default scope is the entire workspace, even when invoked from a member.
`-p` limits both tests and mutations to one package. By default the first 50
mutants are analyzed in generation order; the report shows the selected and
total counts. This is a bounded experiment, not a representative random sample.

To install from source instead:

```sh
git clone https://github.com/yasuyuky/cargo-test-prune.git
cd cargo-test-prune
cargo install --path . --locked
```

From the repository checkout, run the included small example without installing
this binary:

```sh
cargo run -- analyze \
  --manifest-path tests/fixtures/overlap/Cargo.toml \
  --output target/demo-report.json
```

With cargo-mutants 27.1.0, the example discovers six tests (one ignored) and five
mutants. It proposes removing three tests while retaining two active tests that
together detect all five mutants, plus the ignored test. In particular, `zero`
is retained because it alone catches the change from `> 0` to `>= 0`.

The example's negative-input test is a candidate under these five mutations.
That does **not** establish that negative-input coverage is dispensable: its
specification value or a different mutation operator may justify keeping it.

## Inspect and adjust the proposal

```sh
# Keep tests documenting an important requirement (substring of full identity).
cargo test-prune analyze --keep negative --keep backward_compatibility

# Reuse observations without rebuilding or running tests.
cargo test-prune report test-prune.json --keep negative
cargo test-prune report test-prune.json --weight time

# Export one Mermaid diagram per removal candidate, without running tests.
cargo test-prune report test-prune.json --format mermaid > test-prune.md
```

`--keep` is repeatable and errors if a pattern matches no tests. `report` computes
a new proposal for display; it does not overwrite the saved JSON. Pass any pins
again when replaying a report. The default weight is one per test. `--weight time`
uses baseline wall time, including Cargo startup overhead, so results can vary.

Each candidate lists retained tests whose combined kills cover its kills.
**The candidates form one combined proposal**: identical tests are never both
removed if doing so would lose a detected mutant. Greedy weighted set cover,
followed by removal of redundant selections, produces a small set; it does not
guarantee a globally optimal solution.

`report --format mermaid` writes Markdown with one Mermaid flowchart per
candidate. Each diagram shows the candidate's detected mutants and the retained
tests that cover them. Solid arrows show retained tests' detections; dotted
arrows show the candidate's detections. Mutant labels include their one-based
report numbers and mutation descriptions. Unrelated mutants and tests are omitted.
The diagrams reflect `--keep` and `--weight`, just like the default text report;
the saved JSON is unchanged. Open the Markdown in a Mermaid-capable viewer.

## How measurement works

1. Copy the workspace, including uncommitted files, to a temporary directory.
2. Build with `cargo test --tests --no-run --message-format=json`, then enumerate
   each libtest target. Package, target kind, target name, and test name identify
   a test; names in different targets are not merged.
3. Run each exact test separately on the unchanged copy. A failed, timed-out,
   crashed, or unrecognized baseline stops analysis.
4. Generate mutations using `cargo mutants --list --json --no-config`. Apply
   each source span in the copy, rebuild, and run every active test separately.
5. Select a retained set and save observations and the proposal to
   `test-prune.json` (override with `--output`).

Tests run through Cargo so package working directories, runtime library paths,
and Cargo-provided environment variables are preserved. Feature flags
`--features`, `--all-features`, and `--no-default-features` apply to generation
and test execution. cargo-mutants configuration is deliberately disabled so
hidden feature or package settings cannot disagree with the measured tests.

Only an ordinary libtest failure for exactly one test is a kill. Build failures
are **unviable**; build timeouts and incomplete test runs are excluded from the
set-cover universe. Ignored tests, tests with inconclusive observations, and
tests with no observed kills are retained. A timeout never counts as a kill.
The defaults are 30 seconds per test command and 300 seconds per build or
discovery command (`--timeout` and `--build-timeout`). Unix timeouts terminate
the child process group; Ctrl-C cancels analysis and cleans up the copy.

The JSON stores the feature selection, baseline outcomes and durations,
mutations and their spans, per-test outcomes, diagnostics for excluded runs,
and retained witnesses. Mutant numbers in recommendations are **one-based
indices** into `analysis.mutants`; outcome arrays use `analysis.tests` order.

For repeatable experiments, a fresh mutation list can be supplied directly:

```sh
cargo mutants --list --json --no-config > mutants.json
cargo test-prune analyze --mutants-file mutants.json
```

Generate that file from the same source revision, package scope, and features.
Its paths are relative to the workspace root, and its spans must match the
current sources. The tool validates paths and spans, but cannot establish the
provenance of an imported list. The JSON listing interface is documented in
[cargo-mutants' listing guide](https://mutants.rs/list.html).

## MVP scope

- Standard libtest unit and integration tests in self-contained workspaces.
  Doctests, benchmarks, custom harnesses, and nextest are not analyzed.
- Paths in manifests must be relative; external path dependencies, target
  sources outside the workspace, and symlinks are unsupported. The copy omits
  `.git`, directories named `target`, and Cargo's configured target directory.
  Git-dependent builds and configuration inherited from parent directories
  may need adaptation. Ordinary files, including ignored fixtures, are copied.
- Compilation and tests execute project code with the user's normal access.
  The temporary copy isolates mutation edits; it is not a security sandbox.
- Measurements run serially and cost roughly mutants × tests invocations.
  No cache, parallel scheduling, LOC estimate, deletion patch, or automatic
  source editing is included.
- Evidence covers the selected mutations and exact individual runs. It does
  not prove future bug detection, test independence, or absence of flakiness.
  Review specification documentation and diagnostic value before deleting a
  candidate. A separate held-out mutation set is a useful next experiment.

## Development

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

GitHub Actions runs these checks on Ubuntu with stable Rust for pull requests
and pushes to `main`. The CI workflow can also be run manually.

Tests cover duplicate detection, unique kills, weights, pins, and incomplete
observations, and execute real Cargo fixtures for combined removal, ignored
tests, compile failures, and workspace and feature selection. On Unix, a live
descendant holds a local socket open so the interruption test also verifies
process-group termination and temporary workspace cleanup. The test suite uses
explicit mutation lists and does not require cargo-mutants. The example above
exercises the actual cargo-mutants integration.

## Publishing

After running the development checks above, inspect and verify the package:

```sh
cargo package --list --locked
cargo publish --dry-run --locked
```

The archive includes the CLI sources, README, license, and Cargo metadata.
Integration tests and their nested fixture package remain in the repository.
Run the checks from a clean, committed checkout before publishing. After
authenticating with crates.io, publish with `cargo publish --locked` and tag the
published commit as `v0.1.0`.

## License

Licensed under the [MIT License](LICENSE).
