# CLAUDE.md

Guidance for AI agents working in this repo. Human setup (Daml SDK, protoc,
1Password secrets, config files) is in **[README.md](README.md)** — read it for
anything about running against real ledgers; this file covers how to build,
test, and land changes.

## What this is

A Rust workspace for interacting with Canton/Daml ledgers: streaming events,
generating Rust types from Daml models, submitting commands, and syncing/visualizing
ledger state in Neo4j. Rust is a deliberately unsupported client language here —
the point is to understand the Ledger API and `.dalf` encoding from scratch.

## Workspace map

| Crate | Role |
|---|---|
| `ledger-api` | Generated gRPC/protobuf bindings for the Canton Ledger API v2 (tonic/prost). `build.rs` compiles vendored protos under `ledger-api/resources/protobuf` into `src/pb/` **on every build**. |
| `codegen` | Generates Rust types from `.dar` files. `build.rs` downloads the Daml LF proto for the version in `codegen/daml-sdk-version` from GitHub releases. Checked-in output lives in `codegen/generated/`. |
| `daml-type-rep` | Daml type representations: built-in types, `LapiAccess`/`ToCreateArguments` traits, template IDs, numeric scale. |
| `derive-lapi-access` | Proc-macros deriving `LapiAccess` / `ToCreateArguments`. |
| `client` | Ledger API client helpers: active contracts, JWT, party/user management, DAR upload, run-script. |
| `submit` | Command submission helpers (`create_contract`, `exercise_choice`). |
| `sandbox-init` | Starts a Daml sandbox, uploads DARs, runs an init script. |
| `test` (`daml-model-rep-examples`) | Hand-written **integration** examples that exercise generated types against a live sandbox. |
| `event-graph-indexer` | CLI that syncs ACS/updates into Neo4j (Keycloak auth, 1Password). |
| `event-graph-ui` | Dioxus UI visualizing the event graph (needs Neo4j). |
| `wallet` | Placeholder crate (stub). |

## Build quirks (read before touching the build)

- **`protoc` is required.** `ledger-api` and `codegen` build scripts invoke it.
  `codegen/build.rs` also makes a network call to download the matching Daml LF
  proto on a clean build.
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
commented `#[allow(...)]` (see `ledger-api/src/lib.rs`, `test/src/lib.rs`,
`event-graph-ui/src/main.rs`).

## Tests

`cargo test --no-run` only **compiles** tests. The integration tests in `test/`
and `submit/`, and `event-graph-indexer`'s graph/sync tests, need a live Daml
sandbox / Neo4j (and DARs built with `dpm`), so they are not run in CI. Run them
manually via the `justfile` (e.g. `just test-nested`, `just sandbox-init-*`).
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
  `event-graph-indexer/config/config.toml`, `event-graph-ui/config.toml`,
  `secrets.env.1password`, `.env`.
