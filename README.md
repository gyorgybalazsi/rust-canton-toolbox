# canton-event-graph-syncer

Syncs a Canton ledger's event graph into [SurrealDB](https://surrealdb.com).
The `syncer` program streams transactions, created contracts, exercised choices
and parties from the Ledger API v2 and writes them as a graph: tables for the
nodes, relation tables for the edges, and contract and choice arguments as
nested objects you can query by path. It loads the Active Contract Set first,
follows the ledger in real time, and resumes from the last stored offset.

The workspace also holds the Rust Ledger API client libraries the syncer is
built on.

See also: [A Daml ledger tells a story](https://discuss.daml.com/t/blog-post-a-daml-ledger-tells-a-story-it-s-time-to-show-it-to-everyone/6734).

## Prerequisites

- **[protoc](https://protobuf.dev/installation/)** (Protocol Buffer Compiler)
- **[SurrealDB](https://surrealdb.com/install)** `4.x` server for the syncer
  (the `surreal` program also runs the syncer's integration tests)
- **Daml SDK** `3.7.1+` (`dpm install 3.7.1`), for a local sandbox and the Daml examples
- **[just](https://github.com/casey/just)** command runner (optional, for convenience recipes)

Add protoc to rust-analyzer in `.vscode/settings.json`:

```json
{
    "rust-analyzer.server.extraEnv": {
        "PROTOC": "/usr/local/bin/protoc"
    }
}
```

## Quick start

1. Set up a SurrealDB namespace, database and user for the syncer (see
   [syncer README → SurrealDB](syncer/README.md#surrealdb)).
2. Copy `syncer/config/config.toml.example` to `syncer/config/config.toml`
   (gitignored) and fill in your ledger profiles and database settings.
3. Provide the secrets (next section).
4. Run it:

   ```
   just sync-sandbox      # against a local sandbox, with a fake JWT
   just sync-run          # against a remote ledger, with Keycloak auth
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
`syncer/config/config.toml`, starting from
`syncer/config/config.toml.example`. The SurrealDB connection
settings can also come straight from `SURREAL_*` environment variables (see the
[syncer README](syncer/README.md#configuration)).

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

## Running

Common tasks are available as `just` recipes:

```
just                           # list all recipes
just sync-run                  # sync with Keycloak auth (release mode)
just sync-fresh                # fresh start: clear synced data + reload ACS
just sync-sandbox              # sync against local sandbox (fake JWT)
just sync-sandbox-fresh        # fresh start against local sandbox
just sync-stop                 # stop the syncer
just get-update <update-id>    # look up one update by its ID
just build-dars                # build every example DAR in _daml/
just test-sandbox              # build the DARs, run the sandbox integration tests
just test-nested               # run only the nested-test integration test
just sandbox-init-ticketoffer  # init sandbox with ticketoffer example
```

Without `just`, run the syncer with `cargo run --release -p syncer -- sync ...`.
Its subcommands are `sync`, `benchmark` (raw stream throughput, no database
writes) and `print-surql` (print the SurrealQL writes without a database); see
the [syncer README](syncer/README.md#commands) for their options and for the
data model and example queries.

## Workspace Crates

| Crate | Type | Description |
|-------|------|-------------|
| **syncer** | bin/lib | Streams Canton events into SurrealDB as a graph, with resilient sync, batching, and nested contract arguments |
| **ledger-api** | lib | Generated Protobuf/gRPC bindings for the Daml Ledger API v2 |
| **client** | bin/lib | CLI and library wrapping Ledger API operations (streaming, JWT, party management) |
| **daml-type-rep** | lib | Type representations for Daml values (built-in types, numeric scaling, template IDs) |
| **derive-lapi-access** | proc-macro | Derive macro implementing the `LapiAccess` trait for gRPC type conversions |
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
| `get-update-by-id` | Look up a single update (transaction) by its ID |

Run `cargo run -p client -- <subcommand> --help` for parameter details.

### Generating Rust types from DARs

The DAR-to-Rust code generator lives in its own repository,
[daml-rust-codegen](https://github.com/gyorgybalazsi/daml-rust-codegen). It
produces structs that use this workspace's `daml-type-rep`,
`derive-lapi-access`, and `ledger-api` crates. The output for the example DARs
used by the `test` crate is checked in under `test/generated/`; regenerate it
with that tool when the Daml examples change.

## Daml Examples

Example Daml models live in `_daml/`. Each is a multi-package project (`main`,
usually `test`, sometimes `interfaces`); build one with `dpm build --all` in its
folder, or all of them with `just build-dars`.

`just sandbox-init-<example>` (for example `just sandbox-init-ticketoffer`)
builds an example, starts a sandbox with its DAR, and runs its setup script,
which allocates the parties. To get the party IDs afterwards, open the Canton
console with `dpm canton-console` and run
`sandbox.parties.list().map(_.party.toProtoPrimitive)`.

## Tests

CI only compiles the tests. To run them locally:

- `cargo test -p syncer` runs the syncer's unit tests, plus its SurrealDB
  integration test when the `surreal` program is on your PATH (it starts its
  own in-memory server and skips itself otherwise).
- `just test-sandbox` runs the integration tests in `test`, `client` and
  `submit`. They start their own Daml sandbox, so they need the example DARs
  (built by the recipe) and free sandbox ports: stop any sandbox you have
  running first, or every test fails with "Failed to bind to address". The
  tests share fixed ports, which is why the recipe runs them with
  `--test-threads=1`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
