# Tabularis MongoDB Plugin

A [Tabularis](https://github.com/TabularisDB/tabularis) database driver plugin for
**MongoDB 6.0+**, written in Rust on top of the official
[`mongodb`](https://crates.io/crates/mongodb) driver. It speaks Tabularis'
JSON-RPC-over-stdio protocol.

Unlike SQL drivers, the query editor accepts **native MongoDB shell calls**
(`db.collection.find(...)`, `aggregate`, CRUD, ...) rather than SQL.

## Requirements

- A reachable MongoDB 6.0+ server (local, self-hosted, or Atlas).
- A Rust toolchain to build from source. **No system libraries needed** — the
  driver is pure Rust with bundled TLS (rustls), so builds are clean on
  Windows, Linux and macOS.

## Building

```sh
cargo test                # unit tests (no database needed)
cargo build --release     # -> target/release/tabularis-mongodb-plugin(.exe)
```

## Installing

Copy the manifest and binary into a folder named `mongodb` inside the Tabularis
plugins directory, then restart Tabularis (or enable it under
**Settings → Plugins → Installed**):

- **Windows:** `%APPDATA%\debba\tabularis\data\plugins\mongodb\`
- **Linux:** `~/.local/share/tabularis/plugins/mongodb/`
- **macOS:** `~/Library/Application Support/tabularis/plugins/mongodb/`

```
mongodb/
├── manifest.json
└── tabularis-mongodb-plugin(.exe)
```

## Connecting

Fill the connection form (host, port, user, password). MongoDB is **multi-database**:
you connect once and browse every database on the server; collections appear as
"tables". Use the plugin settings (gear icon) for anything beyond host/port:

| Setting | Purpose |
|---|---|
| Connection String | Full `mongodb://` / `mongodb+srv://` URI, used verbatim (overrides host/port). |
| Use SRV | DNS seedlist scheme (`mongodb+srv://`), typical for Atlas. |
| Auth Source | `authSource`, e.g. `admin`. |
| Replica Set | `replicaSet` name. |
| Enable TLS | Connect over TLS. |
| Schema Inference Sample Size | Documents sampled per collection to infer columns (default 100). |
| Extra Connection Options | Verbatim query params, e.g. `retryWrites=true&w=majority`. |

You can also paste a full connection string into the connection-string import field.

## Querying (native MongoDB)

The query editor accepts shell-style calls. Argument bodies are JSON / Extended
JSON; the shell helpers `ObjectId("..")`, `ISODate("..")`, `NumberLong(..)`,
`NumberDecimal("..")` and `NumberInt(..)` are understood.

```js
db.users.find({ "age": { "$gt": 30 } }, { "name": 1, "_id": 0 })
db.users.find({ "_id": ObjectId("507f1f77bcf86cd799439011") })
db.sales.aggregate([{ "$match": { "region": "MX" } }, { "$group": { "_id": "$city", "n": { "$sum": 1 } } }])
db.users.countDocuments({ "active": true })
db.users.distinct("country", {})
db.users.insertOne({ "name": "Ada", "active": true })
db.users.updateMany({ "active": false }, { "$set": { "archived": true } })
db.users.deleteOne({ "_id": ObjectId("507f1f77bcf86cd799439011") })
db.createCollection("audit_log")
db.runCommand({ "dbStats": 1 })
```

`find` and `aggregate` are paginated by the grid automatically. Clicking a
collection in the explorer runs the equivalent of `db.coll.find({})`.

## Feature coverage

| Area | Status |
|---|---|
| Connect / ping / test_connection | ✅ |
| List databases, collections (as tables) | ✅ |
| Column inference by sampling (nested fields → JSON) | ✅ |
| Indexes (list, create, drop) | ✅ |
| Views (list + definition) | ✅ |
| Native queries: find / aggregate / count / distinct | ✅ |
| Document CRUD: insert / update / delete / replace | ✅ |
| Inline grid editing (by `_id`, ObjectId-aware) | ✅ |
| Create / drop collection | ✅ (via `db.createCollection(...)` / `db.coll.drop()`) |

## Known limitations / mapping notes

- **Schemaless ⇒ no column management.** `manage_tables` is off; there are no
  fixed columns to ALTER. Create/drop collections via native commands.
- **No foreign keys or stored routines** — those metadata calls return empty.
- **Column inference is sampled**, so rarely-used fields may be missed; raise the
  sample size if needed. Conflicting scalar types across a field report as `MIXED`.
- Decimal128/dates/ObjectId render as strings in the grid to stay readable.

## License

Apache-2.0.
