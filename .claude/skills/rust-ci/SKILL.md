---
name: rust-ci
description: Run the canton-toolbox CI gate locally before pushing — cargo fmt --check, build, test --no-run, and clippy -D warnings under the pinned Rust toolchain. Use when asked to pre-flight CI, check a Rust change before pushing/opening a PR, or reproduce the required "build / test / clippy / fmt" check. Mirrors .github/workflows/ci.yml exactly.
---

# Rust CI gate (local pre-flight)

Reproduce the repo's **required** GitHub Actions check (`build / test / clippy / fmt`)
locally so a push doesn't get rejected by the `main` ruleset. This mirrors
`.github/workflows/ci.yml` step-for-step.

## Prerequisites
- `protoc` on PATH (the tonic/prost build scripts need it).
  Install on macOS with `brew install protobuf`.
- The Rust version is pinned by `rust-toolchain.toml` (currently 1.96.0), so a
  plain `cargo` already uses the same compiler/clippy as CI. Do **not** override
  it with `+stable`/`+nightly` — version drift is exactly what this gate guards against.

## Run the gate (same order as CI; stop at the first failure)
Run from the repo root. `--locked` is intentional — CI uses it, so it catches
`Cargo.lock` drift too.

```bash
cargo fmt --all --check
cargo build --workspace --all-targets --locked
cargo test --workspace --no-run --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`build` and `clippy` recompile the workspace, so a cold run takes several minutes.

## How to interpret / fix
- **fmt fails** -> run `cargo fmt --all` to fix, then re-run the gate.
- **clippy fails** -> fix the lint at the source. Use `cargo clippy --fix` for
  machine-applicable suggestions; for generated code (`ledger-api/src/pb/*`) or
  intentional patterns, prefer a scoped `#[allow(...)]` with a comment over
  editing — see existing examples in `ledger-api/src/lib.rs` and `test/src/lib.rs`.
- **build/test fails** -> a real compile error; fix it.

## What this does NOT do
`cargo test --no-run` **compiles** every test target but runs nothing. The
integration tests in `test/` and `submit/` need a live DAML sandbox that isn't
available in CI — run those manually via the `justfile` recipes (e.g.
`just test-nested`) when needed. `canton-event-graph-sync`'s SurrealDB tests start
their own throwaway `surreal` server: `cargo test -p canton-event-graph-sync`.

## Done when
All four commands exit 0 — the same condition the required CI check enforces, so
the PR will be mergeable (assuming signed commits + an open PR).
