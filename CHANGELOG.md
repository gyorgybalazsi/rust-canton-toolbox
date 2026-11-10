# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed
- Daml examples in `_daml/` and codegen's Daml-LF protos moved to SDK 3.7.1
  (`daml-nested-test` was on 3.4.11, the rest on 3.4.8)
- Vendored Ledger API v2 protos bumped from Canton 3.6.1 to 3.7.1 (adds
  `size_config_options.proto`, `account_id` traffic fields, and
  `include_transactions_and_reassignments` / `include_incomplete_reassignments`)
- ACS readers (`active_contracts`, `get_blob`) set
  `include_incomplete_reassignments = true` so incomplete (un)assigned entries
  are still returned, as before 3.7
- `event-graph-indexer` writes to a SurrealDB 4 server instead of Neo4j.
  Neo4j labels became tables (`transaction`, `created`, `exercised`, `party`,
  `sync_state`) and relationship types became relation tables (`action`,
  `consequence`, `target`, `consumes`, `requested`). Contract and choice
  arguments are stored as nested objects instead of flattened properties or
  JSON strings; party lists and result contract ids are arrays; timestamps are
  datetimes. See `event-graph-indexer/README.md`.
- Config: the `[neo4j]` section is replaced by `[surrealdb]`
  (`url`, `namespace`, `database`, `username`, `password`), each overridable
  with a `SURREAL_*` environment variable. The indexer signs in as a database
  user, never as root.
- Database writes retry with a growing delay (1 s up to 30 s) while the server
  is down, keeping the batch in memory instead of failing.
- `print-cypher` is now `print-surql`.

### Removed
- `event-graph-ui` (the Dioxus graph visualization).
- The `[storage]` config section (`flatten_arguments`, `flatten_max_depth`,
  `store_arguments_json`): arguments are always stored nested.
- The `neo4rs` dependency.

## [0.1.0] - 2026-07-01

### Added
- Dual-licensed under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`)
- Flatten create/choice arguments into dot-separated Neo4j node properties
  (e.g., `create_arg.person.homeAddress.city = "Zurich"`) for direct querying
- Configurable `[storage]` section in config.toml: `flatten_arguments`,
  `flatten_max_depth`, `store_arguments_json`
- Auto-discover parties from ledger when none configured (`ListKnownParties`)
- `store_arguments_json` flag to optionally store raw JSON blobs alongside
  flattened properties (default: false)

### Changed
- Renamed crate `ledger-explorer` → `event-graph-indexer` and `ledger-graph-ui`
  → `event-graph-ui` to better reflect the Canton event-graph functionality
  (binary, config-discovery paths, and `-p` package references updated to match)
- Renamed justfile recipes: `indexer-run`, `indexer-sandbox`,
  `indexer-fresh`, `indexer-sandbox-fresh`, `indexer-stop`
- `PrintCypher` and `Benchmark` commands now respect storage config instead of
  using hardcoded flatten settings

### Removed
- Removed the placeholder `wallet` crate (an empty `Hello, world!` stub with no
  dependents) from the workspace
