# CLAUDE.md

Guidance for AI agents working in this repo. Human setup (Daml SDK, protoc,
1Password secrets, config files) is in **[README.md](README.md)** — read it for
anything about running against real ledgers; this file covers how to build,
test, and land changes.

## What this is

A Rust workspace for interacting with Canton/Daml ledgers: streaming events,
using Rust types generated from Daml models, submitting commands, and syncing the
ledger event graph into SurrealDB. Rust is a deliberately unsupported client language here —
the point is to understand the Ledger API and `.dalf` encoding from scratch.

## Workspace map

| Crate | Role |
|---|---|
| `ledger-api` | Generated gRPC/protobuf bindings for the Canton Ledger API v2 (tonic/prost). `build.rs` compiles vendored protos under `ledger-api/resources/protobuf` into `src/pb/` **on every build**. |
| `daml-type-rep` | Daml type representations: built-in types, `LapiAccess`/`ToCreateArguments` traits, template IDs, numeric scale. |
| `derive-lapi-access` | Proc-macros deriving `LapiAccess` / `ToCreateArguments`. |
| `client` | Ledger API client helpers: active contracts, JWT, party/user management, DAR upload, run-script. |
| `submit` | Command submission helpers (`create_contract`, `exercise_choice`). |
| `sandbox-init` | Starts a Daml sandbox, uploads DARs, runs an init script. |
| `test` (`daml-model-rep-examples`) | Hand-written **integration** examples that exercise generated types against a live sandbox. The generated types are checked in under `test/generated/`; they come from the standalone code generator at https://github.com/gyorgybalazsi/daml-rust-codegen. |
| `syncer` | CLI that syncs ACS/updates into a SurrealDB server (Keycloak auth, 1Password). |
| `wallet` | Placeholder crate (stub). |

## Build quirks (read before touching the build)

- **`protoc` is required.** The `ledger-api` build script invokes it.
- **`ledger-api/src/pb/*.rs` is generated** by `build.rs` on every compile —
  never hand-edit it. Suppress lints on it via `#[allow(...)]` on the including
  modules in `ledger-api/src/lib.rs`, not in the generated files.
- **Toolchain is pinned** in `rust-toolchain.toml` (Rust 1.96.0 + clippy + rustfmt).
  Don't override with `+stable`/`+nightly`; version drift breaks the CI gate.

## CI gate (required to merge)

Every PR must pass the **required** GitHub Actions check `build / test / clippy / fmt`
(`.github/workflows/ci.yml`). It runs, in order:

```bash
cargo fmt --all --check
cargo build --workspace --all-targets --locked
cargo test --workspace --no-run --locked      # compiles tests; does NOT run them
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Reproduce it locally before pushing — the `/rust-ci` skill does exactly this.
Keep clippy clean; for generated code or intentional patterns use a scoped,
commented `#[allow(...)]` (see `ledger-api/src/lib.rs`, `test/src/lib.rs`).

## Tests

`cargo test --no-run` only **compiles** tests. The integration tests in `test/`
and `submit/` need a live Daml sandbox (and DARs built with `dpm`), so they are
not run in CI. `syncer/tests/surreal_integration.rs` starts its own
throwaway in-memory server with the `surreal` program and skips itself when that
isn't installed: run it with `cargo test -p syncer`. Run the
sandbox ones manually with `just test-sandbox` (builds the DARs, runs them with `--test-threads=1`; no other sandbox may be running). See also `just test-nested` and `just sandbox-init-*`.
See `justfile` for all recipes.

## Landing changes (branch protection on `main`)

`main` is protected by a ruleset that requires:
- a **pull request** (no direct pushes),
- **signed/verified commits** (SSH or GPG commit signing must be configured),
- **passing CI** (the gate above),
- no force-push, no branch deletion.

So: branch → commit (signed) → open PR → CI green → merge. Use `cargo fmt` and
the `/rust-ci` skill before pushing to avoid a red CI.

## Conventions

- Commit `Cargo.lock`; CI builds with `--locked`.
- Secrets come from the environment (1Password `op run` by default, see README);
  never commit secret values or real `op://` item paths. Only the generic
  `secrets.env.1password.example` is tracked; `secrets.env.1password` and `.env`
  are gitignored.
- Local-only files (gitignored): `CLAUDE.local.md`, `.claude/` (except the
  shared `.claude/settings.json` and `.claude/skills/`),
  `syncer/config/config.toml`, `secrets.env.1password`, `.env`.
