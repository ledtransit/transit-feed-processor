# GTFS Transit Feed Processor

A small Rust service that keeps a PostgreSQL database in sync with public transit
data:

- **Static schedules (GTFS):** every 30 minutes it downloads the configured GTFS
  feeds (skipping unchanged ones via `ETag` or content hash), filters them into
  named *scopes* (e.g. only Berlin S-Bahn and U-Bahn routes), and mirrors the
  selected routes, trips, stop times, stops and calendars into Postgres.
- **Realtime (GTFS-Realtime):** every 30 seconds it polls the GTFS-RT feeds,
  matches their trip updates against the trips that could be running right now,
  and stores per-trip and per-stop delays, cancellations and skipped stops in
  `rt_trip_update` / `rt_stop_time_update`. Rows keep their last known state
  when a feed briefly drops a trip, and are pruned once their service day is over.
- After each realtime poll it sends a Postgres `NOTIFY` on the
  `transit_realtime_updated` channel (payload: the realtime source idents), so
  downstream consumers can react without polling.

The database schema is created automatically on startup from
[`migrations/`](migrations/).

## Feeds

Feeds and their scopes are configured in code under [`src/feed/`](src/feed/). The
defaults cover Berlin S-Bahn/U-Bahn and German long-distance rail:

| Feed      | Static GTFS                                             | Realtime                                       |
| --------- | ------------------------------------------------------- | ---------------------------------------------- |
| `vbb`     | [VBB](https://www.vbb.de/vbbgtfs) (CC BY 4.0)           | VBB GTFS-RT (CC BY 4.0)                        |
| `delfi`   | [DELFI](https://data.public-transport.earth/gtfs/de) (CC BY 4.0) | DELFI GTFS-RT via Mobilithek (CC BY-SA 4.0, credentials required) |
| `gtfs_de` | [gtfs.de](https://gtfs.de) (CC BY 4.0)                  | gtfs.de realtime (CC BY-SA 4.0)                |

To add a feed, create a module in `src/feed/` describing its sources, scopes
and timezone, and register it in `FEEDS` in [`src/feed/mod.rs`](src/feed/mod.rs).
Please respect each data provider's license and terms of use.

## Setup

### Requirements

- Rust 1.94 or newer
- Protobuf compiler (`protoc` in `PATH`)
- PostgreSQL (the `btree_gist` extension is enabled by the migrations)

### Configuration

Copy `.env.example` to `.env` and fill it in:

| Variable                                   | Required | Description |
| ------------------------------------------ | -------- | ----------- |
| `DATABASE_URL`                             | yes      | Postgres connection string |
| `USER_AGENT`                               | yes      | User-Agent for feed requests; must identify you (contact URL or email) |
| `MOBILITHEK_DELFI_GTFS_RT_SUBSCRIPTION_ID` | no       | Mobilithek subscription for the DELFI realtime feed |
| `MOBILITHEK_M2M_CERT_PASSWORD`             | no       | Password of the client certificate at `cert/mobilithek.p12` |

Sources whose credentials are missing are skipped with a warning.
Downloaded feeds are cached in `data/` in the working directory. Log
verbosity is controlled with `RUST_LOG` (default `info`).

### Run

```bash
cargo run --release
```

### Docker

```bash
docker build -t transit-feed-processor .
docker run --env-file .env -v "$PWD/data:/app/data" -v "$PWD/cert:/app/cert:ro" transit-feed-processor
```

Inside the container, `DATABASE_URL` must point to a host the container can
reach (not `localhost`).

### Tests

```bash
cargo test
```

The database-backed tests use `#[sqlx::test]` and need `DATABASE_URL` to point
at a Postgres user with permission to create databases.

## Plugins

The optional `plugin` Cargo feature compiles in additional realtime sources
from a `proc-plugin/` directory next to this crate. It is not part of this
repository and the default build doesn't need it.

## License

The code is licensed under the [MIT License](LICENSE). Feed data fetched at
runtime is subject to the respective providers' licenses (see [Feeds](#feeds)).
