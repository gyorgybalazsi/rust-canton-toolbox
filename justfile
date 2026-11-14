# Load a local, gitignored `.env` if present (values, or SECRETS_RUNNER/OP_ACCOUNT).
set dotenv-load

# Command prefix that injects secrets into recipes that need them.
# Default: 1Password, resolving the op:// references in secrets.env.1password.
# Set OP_ACCOUNT to pin a 1Password account (e.g. my.1password.com) when
# several are signed in. Set SECRETS_RUNNER to use something else, or to an
# empty value when the variables are already in the environment or `.env`.
op_account_flag := if env("OP_ACCOUNT", "") == "" { "" } else { "--account " + env("OP_ACCOUNT", "") + " " }
secrets_runner := env("SECRETS_RUNNER", "op run " + op_account_flag + "--env-file=secrets.env.1password --")

# List available recipes
default:
    @just --list

# Run the sync with Keycloak authentication (release mode)
sync-run:
    {{secrets_runner}} \
      cargo run --release -p syncer -- sync --use-keycloak 2>&1 | tee sync.log

# Fresh start: delete the synced data in SurrealDB, load current ACS, and stream from ledger end
sync-fresh:
    {{secrets_runner}} \
      cargo run --release -p syncer -- sync --use-keycloak --fresh 2>&1 | tee sync.log

# Run the sync against local sandbox (no Keycloak, fake JWT)
sync-sandbox:
    {{secrets_runner}} \
      cargo run --release -p syncer -- sync --profile local 2>&1 | tee sync.log

# Fresh start against local sandbox (no Keycloak)
sync-sandbox-fresh:
    {{secrets_runner}} \
      cargo run --release -p syncer -- sync --profile local --fresh 2>&1 | tee sync.log

# Stop the sync
sync-stop:
    pkill -x syncer || true

# Look up a single update (transaction) by its ID
get-update update_id profile="mainnet":
    {{secrets_runner}} \
      cargo run -p client -- get-update-by-id --profile {{profile}} --update-id {{update_id}}

# --- Integration tests ---

# Run the nested-test integration test (starts sandbox, creates contract, exercises choice)
test-nested:
    cd _daml/daml-nested-test && dpm build --all
    cargo test -p daml-model-rep-examples nested_test -- --nocapture

# --- Sandbox init recipes ---
# Each builds the DAML packages and runs sandbox-init

sandbox-init-asset:
    cd _daml/daml-asset && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/daml-asset/main/.daml/dist/daml-asset-0.0.1.dar \
        --init-dar _daml/daml-asset/test/.daml/dist/daml-asset-test-0.0.1.dar \
        --init-script-name "Test:setup"

sandbox-init-interface:
    cd _daml/daml-interface-example && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/daml-interface-example/main/.daml/dist/daml-interface-example-main-1.0.0.dar \
        --init-dar _daml/daml-interface-example/test/.daml/dist/daml-interface-example-test-1.0.0.dar \
        --init-script-name "Test:test"

sandbox-init-optional:
    cd _daml/daml-optional && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/daml-optional/main/.daml/dist/daml-optional-0.0.1.dar \
        --init-dar _daml/daml-optional/test/.daml/dist/daml-optional-test-0.0.1.dar \
        --init-script-name "Test:setup"

sandbox-init-ticketoffer:
    cd _daml/daml-ticketoffer && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/daml-ticketoffer/main/.daml/dist/daml-ticketoffer-0.0.1.dar \
        --init-dar _daml/daml-ticketoffer/test/.daml/dist/daml-ticketoffer-test-0.0.1.dar \
        --init-script-name "Test:setup"

sandbox-init-ticketoffer-explicit-disclosure:
    cd _daml/daml-ticketoffer-explicit-disclosure && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/daml-ticketoffer-explicit-disclosure/main/.daml/dist/daml-ticketoffer-explicit-disclosure-0.0.1.dar \
        --init-dar _daml/daml-ticketoffer-explicit-disclosure/test/.daml/dist/daml-ticketoffer-explicit-disclosure-test-0.0.1.dar \
        --init-script-name "Setup:setup"

sandbox-init-full:
    cd _daml/full && dpm build --all
    cargo run --bin sandbox-init -- \
        --dar _daml/full/main/.daml/dist/full-0.0.1.dar \
        --init-dar _daml/full/test/.daml/dist/full-test-0.0.1.dar \
        --init-script-name "Test:setup"
