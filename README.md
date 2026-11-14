# Client Toolbox for Canton

## Overview

A Rust workspace for interacting with Canton/Daml ledgers: streaming events, submitting commands with Rust types generated from Daml models, and syncing the ledger event graph into SurrealDB.

Why Rust? Because it's not an officially supported client language, and building from scratch is the best way to understand the Ledger API and `.dalf` encoding.

## Prerequisites

- **Daml SDK** `3.7.1+` (`dpm install 3.7.1`)
- **[protoc](https://protobuf.dev/installation/)** (Protocol Buffer Compiler)
- **[SurrealDB](https://surrealdb.com/install)** `4.x` server (for canton-event-graph-sync; the `surreal` program also runs its integration tests)
- **[just](https://github.com/casey/just)** command runner (optional, for convenience recipes)

Add protoc to rust-analyzer in `.vscode/settings.json`:

```json
{
    "rust-analyzer.server.extraEnv": {
        "PROTOC": "/usr/local/bin/protoc"
    }
}
```

## Secrets

Config files never hold secret values. Secret fields use `${VAR}` placeholders
(e.g. `client_secret = "${MAINNET_CLIENT_SECRET}"`), and the loader fills them in from
environment variables when it reads the config. Any way of setting those
variables works; 1Password is the default the `justfile` assumes.

The variable names are up to you: they only have to match between your
`config.toml` and wherever the values come from. The committed
[`secrets.env.1password.example`](secrets.env.1password.example) lists the
names used by the example configs (`SURREAL_PASSWORD`, `DEVNET_CLIENT_SECRET`,
`MAINNET_CLIENT_SECRET`, ...).

First, create your local config file (gitignored):
`canton-event-graph-sync/config/config.toml`, starting from
`canton-event-graph-sync/config/config.toml.example`. The SurrealDB connection
settings can also come straight from `SURREAL_*` environment variables (see the
[canton-event-graph-sync README](canton-event-graph-sync/README.md#configuration)).

Then choose where the values come from.

### Option A: 1Password (default)

1. Install the 1Password CLI (`brew install --cask 1password-cli`) and run `op signin`.
2. Store each secret as an item in a vault you can access.
3. Copy the template and point each line at your items:

   ```bash
   cp secrets.env.1password.example secrets.env.1password   # gitignored
   ```

   ```bash
   SURREAL_PASSWORD="op://<vault>/<item>/<field>"
   ```

4. Check one reference with `op read 'op://<vault>/<item>/<field>'`.

`just` recipes then run under `op run --env-file=secrets.env.1password --`, which
resolves the references at runtime. If you are signed in to several 1Password
accounts, set `OP_ACCOUNT` (e.g. `OP_ACCOUNT=my.1password.com`) in your shell or `.env`.

### Option B: `.env` file or exported variables

Create a gitignored `.env` in the repo root with real values, and tell `just`
not to wrap commands in `op run`:

```bash
SECRETS_RUNNER=
SURREAL_PASSWORD=...
MAINNET_CLIENT_SECRET=...
```

`just` loads `.env` automatically. When running `cargo run` directly instead,
export the variables yourself.

### Option C: another secret manager

Set `SECRETS_RUNNER` to any command prefix that injects environment variables
and then runs the rest of the command, e.g.
`SECRETS_RUNNER="doppler run --"` or `SECRETS_RUNNER="aws-vault exec dev --"`.

## Workspace Crates

| Crate | Type | Description |
|-------|------|-------------|
| **ledger-api** | lib | Generated Protobuf/gRPC bindings for the Daml Ledger API v2 |
| **client** | bin/lib | CLI and library wrapping Ledger API operations (streaming, JWT, party management) |
| **daml-type-rep** | lib | Type representations for Daml values (built-in types, numeric scaling, template IDs) |
| **derive-lapi-access** | proc-macro | Derive macro implementing the `LapiAccess` trait for gRPC type conversions |
| **canton-event-graph-sync** | bin/lib | Streams Canton events into SurrealDB as a graph, with resilient sync, batching, and nested contract arguments |
| **submit** | lib | Library for submitting Daml contracts and exercising choices via gRPC |
| **sandbox-init** | bin | CLI tool to start a Daml sandbox and run initialization scripts from DAR files |
| **test** | lib | Integration tests for the `LapiAccess` trait paired with Daml examples, using checked-in generated types in `test/generated/` |

### client CLI

```
cargo run -p client -- <subcommand> <params>
```

| Subcommand | Description |
|------------|-------------|
| `get-ledger-end` | Get the ledger end offset |
| `fake-access-token` | Create a fake access token for Sandbox |
| `stream-updates` | Stream ledger updates for a party |
| `stream-transactions` | Stream transactions for a party |
| `parties` | List parties, optionally filtered by substring |

Run `cargo run -p client -- <subcommand> --help` for parameter details.

### canton-event-graph-sync

Streams the Canton event graph into a SurrealDB server with:
- Resilient sync with automatic reconnection and backoff, for both the ledger and the database
- Keycloak OAuth2 authentication (client credentials or password grant)
- Contract and choice arguments stored as nested objects, queryable by path
- ACS (Active Contract Set) bootstrapping
- Configurable batching, flush timeouts, and idle detection
- Multi-profile config support (local, devnet, mainnet)

Configuration goes in `canton-event-graph-sync/config/config.toml` (gitignored).
See the [canton-event-graph-sync README](canton-event-graph-sync/README.md) for the data model.

See also: [A Daml ledger tells a story](https://discuss.daml.com/t/blog-post-a-daml-ledger-tells-a-story-it-s-time-to-show-it-to-everyone/6734).

### Generating Rust types from DARs

The DAR-to-Rust code generator lives in its own repository,
[daml-rust-codegen](https://github.com/gyorgybalazsi/daml-rust-codegen). It produces structs that use this
workspace's `daml-type-rep`, `derive-lapi-access`, and `ledger-api` crates.
The output for the example DARs used by the `test` crate is checked in under
`test/generated/`; regenerate it with that tool when the Daml examples change.

## Running

Common tasks are available as `just` recipes:

```
just                              # list all recipes
just sync-run                  # sync with Keycloak auth (release mode)
just sync-fresh                # fresh start: clear synced data + reload ACS
just sync-sandbox              # sync against local sandbox (fake JWT)
just sync-sandbox-fresh        # fresh start against local sandbox
just sync-stop                 # stop the sync process
just test-nested                  # run nested-test integration test
just sandbox-init-ticketoffer     # init sandbox with ticketoffer example
```

## Daml Examples

Test Daml models live in the `_daml/` folder. Each contains a Daml script that allocates parties when started with `daml start`.

To retrieve party IDs after starting Sandbox:
1. Start the Canton console: `daml canton-console`
2. List parties: `sandbox.parties.list().map(_.party.toProtoPrimitive)`

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
