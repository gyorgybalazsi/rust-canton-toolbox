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
