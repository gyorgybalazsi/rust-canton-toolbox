# Event Graph Indexer

A Rust tool for syncing Daml/Canton ledger data to a Neo4j graph database. It streams ledger updates in real-time and creates a graph representation of transactions, contracts, and their relationships.

## Supported Daml SDK

This app was tested with sdk-version 3.4.8. The ledger sync uses Ledger API v2.

## Features

- **Resilient Sync**: Automatically reconnects on stream errors with exponential backoff
- **Resume Capability**: Resumes from the last processed offset stored in Neo4j
- **ACS Loading**: Loads the Active Contract Set before streaming to ensure all referenced contracts exist
- **JWT Token Management**: Supports Keycloak OAuth2 (client credentials or password flow) with automatic token refresh
- **Fresh Start Mode**: Option to clear Neo4j and start from current ledger end
- **Optimized Performance**: Batched writes and indexed queries for high throughput

## Graph Schema

### Nodes

| Label | Description | Key Properties |
|-------|-------------|----------------|
| `Transaction` | A ledger transaction | `offset`, `update_id`, `command_id`, `effective_at`, `record_time` |
| `Created` | A created contract event | `contract_id`, `template_name`, `signatories`, `offset`, `node_id` |
| `Exercised` | An exercised choice event | `offset`, `node_id`, `choice_name`, `target_contract_id`, `consuming` |
| `Party` | A party on the ledger | `party_id` |

### Relationships

| Type | From | To | Description |
|------|------|----|-------------|
| `ACTION` | Transaction | Created/Exercised | Root-level events in a transaction |
| `CONSEQUENCE` | Exercised | Created/Exercised | Child events resulting from an exercise |
| `TARGET` | Exercised | Created | The contract being exercised |
| `CONSUMES` | Exercised | Created | Contract consumed by a consuming choice |
| `REQUESTED` | Party | Transaction | Party that requested the transaction |

### Indexes

The following indexes are automatically created for optimal query performance:

- `Created(contract_id)` - For TARGET/CONSUMES lookups
- `Created(offset, node_id)` - For CONSEQUENCE edge creation
- `Exercised(offset, node_id)` - For ACTION and CONSEQUENCE edge creation
- `Transaction(offset)` - For resume point queries
- `Party(party_id)` - For party lookups

## Prerequisites

### Neo4j

Download [Neo4J Desktop](https://neo4j.com/download/) and create a local database.

### Canton / Sandbox

You can connect to:
- Canton Sandbox (default settings)
- Canton 3.4+ via Docker
- Canton Network validator node with port forwarding

Adjust the settings in `config/config.toml` accordingly.

## Installation

```bash
cargo build --release -p event-graph-indexer
```

## Configuration

Create a `config/config.toml` file based on the example:

```bash
cp config/config.toml.example config/config.toml
```

### Configuration Options

```toml
[logging]
level = "info"  # debug, info, warn, error

[neo4j]
uri = "neo4j://127.0.0.1:7687"
user = "neo4j"
password = "password"

[ledger]
fake_jwt_user = "alice"  # Used when --use-keycloak is not specified
parties = ["party-id-1", "party-id-2"]  # Party IDs to subscribe to
url = "https://ledger.example.com:5001"

# Optional: Keycloak for real JWT tokens
[keycloak]
client_id = "your-client-id"
token_endpoint = "https://keycloak.example.com/auth/realms/realm/protocol/openid-connect/token"
# For client credentials flow:
grant_type = "client_credentials"
client_secret = "your-secret"
# Or for password flow:
# grant_type = "password"
# username = "user"
# password = "pass"
```

## Commands

### sync

Continuously syncs ledger updates to Neo4j with automatic reconnection.

```bash
# Normal sync (resumes from last offset in Neo4j)
cargo run --release -p event-graph-indexer -- sync --use-keycloak

# Fresh start (clears Neo4j, loads current ACS, streams from ledger end)
cargo run --release -p event-graph-indexer -- sync --use-keycloak --fresh

# Fresh start from ~7 days ago (resolves the offset via the Ledger API, implies --fresh)
cargo run --release -p event-graph-indexer -- sync --use-keycloak --from-days-ago 7
```

Options:
- `--config-file <path>`: Path to config.toml (default: `./config/config.toml`)
- `--use-keycloak`: Use Keycloak for JWT token management
- `--access-token <token>`: Provide a static JWT token
- `--fresh`: Clear database and start from current ledger end
- `--from-days-ago <N>`: Clear database and start a fresh sync from the offset current ~N days ago (resolved by binary search over the Ledger API). Implies `--fresh`.

### benchmark

Measures Canton stream throughput without writing to Neo4j.

```bash
cargo run --release -p event-graph-indexer -- benchmark --use-keycloak --count 10000
```

Options:
- `--count <n>`: Number of updates to process (default: 10000)
- `--begin-offset <n>`: Starting offset (default: pruning offset)

### print-cypher

Prints the generated Cypher queries for debugging.

```bash
cargo run --release -p event-graph-indexer -- print-cypher \
  --access-token <token> \
  --party <party-id> \
  --url <ledger-url> \
  --begin-exclusive <offset>
```

## Using the justfile

Common operations are available via [just](https://github.com/casey/just):

```bash
# Start syncing with Keycloak
just indexer-run

# Fresh start
just indexer-fresh

# Stop the sync process
just indexer-stop
```

## Querying the Graph

Example Cypher queries:

```cypher
// Find all transactions for a party
MATCH (p:Party {party_id: "alice::1220..."})-[:REQUESTED]->(t:Transaction)
RETURN t ORDER BY t.offset DESC LIMIT 10

// Find all exercises on a specific contract
MATCH (e:Exercised)-[:TARGET]->(c:Created {contract_id: "00..."})
RETURN e.choice_name, e.acting_parties, e.consuming

// Trace the full transaction tree
MATCH path = (t:Transaction)-[:ACTION*]->(n)
WHERE t.offset = 12345
RETURN path

// Find all active contracts (not consumed)
MATCH (c:Created)
WHERE NOT (c)<-[:CONSUMES]-()
RETURN c.template_name, count(*) as active_count
```

You can import the saved Cypher query collection from the `config/` folder into Neo4j Desktop.

## Architecture

```
┌─────────────────┐    ┌──────────────┐    ┌─────────────┐
│  Canton Ledger  │───▶│ event-graph- │───▶│   Neo4j     │
│  (gRPC Stream)  │    │ indexer      │    │   Graph DB  │
└─────────────────┘    └──────────────┘    └─────────────┘
        │                     │
        │                     ▼
        │              ┌──────────────┐
        └─────────────▶│  Keycloak    │
           (auth)      │  (OAuth2)    │
                       └──────────────┘
```

1. **Stream Updates**: Connects to Canton's gRPC update stream
2. **Generate Cypher**: Converts ledger events to Neo4j Cypher queries
3. **Batch Write**: Commits batches of 100 updates for throughput
4. **Auto-reconnect**: Handles stream disconnections with exponential backoff
5. **Token Refresh**: Background thread refreshes JWT before expiry

## Performance

With proper indexes, the sync achieves:
- ~2.5-3.5 offsets/second processing rate
- ~400-500ms per batch of 100 updates (~900 queries)
- Can keep up with ledger activity of ~1.3 updates/second

## Troubleshooting

### Sync falling behind

If the sync cannot keep up with incoming updates:
1. Check Neo4j has indexes: `SHOW INDEXES`
2. Increase Neo4j memory allocation
3. Consider running Neo4j on faster storage (SSD)

### Connection errors

The sync automatically reconnects with exponential backoff. Check:
1. Network connectivity to ledger
2. JWT token validity (use `--use-keycloak` for auto-refresh)
3. Neo4j availability

### Duplicate data

The sync uses MERGE operations to prevent duplicates on reconnection. If duplicates appear, verify that the indexes exist and the sync is using the latest code.

## Learn Cypher

You can explore the event graph with Cypher queries. Neo4j Desktop includes a built-in Cypher tutorial.
