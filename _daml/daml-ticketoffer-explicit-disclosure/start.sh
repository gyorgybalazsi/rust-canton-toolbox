#!/bin/bash

# Start sandbox, upload main DAR, and run setup script for daml-ticketoffer-explicit-disclosure
# Usage: ./start.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MAIN_DIR="$SCRIPT_DIR/main"
TEST_DIR="$SCRIPT_DIR/test"
MAIN_DAR="$MAIN_DIR/.daml/dist/daml-ticketoffer-explicit-disclosure-0.0.1.dar"
TEST_DAR="$TEST_DIR/.daml/dist/daml-ticketoffer-explicit-disclosure-test-0.0.1.dar"

LEDGER_HOST="${LEDGER_HOST:-localhost}"
LEDGER_PORT="${LEDGER_PORT:-6865}"

echo "==> Building main package..."
(cd "$MAIN_DIR" && dpm build)

echo "==> Building test package..."
(cd "$TEST_DIR" && dpm build)

echo "==> Starting sandbox with main DAR..."
(cd "$SCRIPT_DIR" && dpm sandbox --dar "$MAIN_DAR" > sandbox.log 2>&1) &
SANDBOX_PID=$!
echo "$SANDBOX_PID" > "$SCRIPT_DIR/sandbox.pid"

# Wait for sandbox to be ready
echo "==> Waiting for sandbox to be ready..."
for i in {1..60}; do
    if grep -q "Canton sandbox is ready." "$SCRIPT_DIR/sandbox.log" 2>/dev/null; then
        echo "==> Sandbox is ready!"
        break
    fi
    if [ $i -eq 60 ]; then
        echo "==> Timeout waiting for sandbox"
        exit 1
    fi
    sleep 1
done

# Get the actual Java process PID listening on the ledger port
SANDBOX_PID=$(lsof -t -i TCP:"$LEDGER_PORT" -sTCP:LISTEN)
echo "$SANDBOX_PID" > "$SCRIPT_DIR/sandbox.pid"

echo "==> Running setup script..."
dpm script \
    --ledger-host "$LEDGER_HOST" \
    --ledger-port "$LEDGER_PORT" \
    --dar "$TEST_DAR" \
    --script-name "Setup:setup"

echo ""
echo "==> Setup complete! Sandbox is running on $LEDGER_HOST:$LEDGER_PORT (PID: $SANDBOX_PID)"
echo "==> Logs: $SCRIPT_DIR/sandbox.log"
echo "==> To stop: kill \$(cat $SCRIPT_DIR/sandbox.pid)"
