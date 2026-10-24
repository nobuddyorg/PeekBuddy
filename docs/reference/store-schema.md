# Store schema v1

The event store keeps one SQLite file per day (ADR 0004). This page describes
the schema in `peekbuddy-store::schema`; the DDL there is the source of truth
and is snapshot-tested.

## Rules

- Every table is `STRICT`; there are no foreign keys and no `AUTOINCREMENT`.
- `PRAGMA application_id` is `0x50424B31` (`PBK1`); `meta.schema_version`
  holds the schema version.
- Event ids are one global, monotonic, contiguous sequence across segments,
  assigned by the writer inside the commit transaction.
- Timestamps are FILETIME ticks (100 ns, UTC).
- Enum values stored in the tables are append-only and never renumbered.
- `TEXT` is always valid UTF-8. Exact UTF-16 names go in a `BLOB` sidecar.
- File ids are two columns: `file_id` (the low 64 bits, bit-cast to a signed
  integer) and `file_id_hi` (the high 64 bits, NULL when zero).

## Dictionary part

| Table | Purpose |
| --- | --- |
| `node` | The interned directory tree: `parent_id` (0 for top level), `name` (lossy UTF-8), `name_w` (exact UTF-16LE, only when the name is not valid Unicode), `name_kind`, `file_id`. |
| `node_fts` | FTS5 trigram index over `node.name`, `detail='none'`, written by the writer, no triggers. |
| `root` | Watch roots. |
| `vol` | Volumes: GUID path, serial, letter, journal id. |
| `proc` | Reserved for process attribution after v1. |

`node_key` is a unique index on `(parent_id, name, coalesce(name_w, x''))`:
SQLite treats NULLs in a unique index as distinct, so the sidecar is
coalesced and the parent is never NULL. Names are case-preserving and
byte-exact: `a.txt` and `A.txt` are different nodes, and two names that
differ only in lone surrogates are different nodes.

Because the FTS table has `detail='none'`, a `MATCH` with more than one
trigram is a phrase query and SQLite rejects it. Substring search uses
`name LIKE '%text%'` on `node_fts`, which the trigram index accelerates and
which folds case for Unicode. Whether `LIKE` on this external-content
table really uses the trigram index (and not a scan of `node`) is unverified
and #55 must check it with `EXPLAIN QUERY PLAN`.

## Segment part

| Table | Purpose |
| --- | --- |
| `event` | One row per event; see below. |
| `stats` | `(hour_ts, root_id, kind)` counts with the id range, one UPSERT per key per batch. |
| `cursor` | Capture checkpoint per `(root_id, vol_id)`. |
| `meta` | Key/value, including `schema_version`. |

`event.kind` packs the wire tag and its payload byte as `tag * 256 + arg`
(`Kind::to_wire`). `event.source` is `Source::tag`, `event.name_kind` lives on
the node. Indexes are `event(node_id, id)` and `event(root_id, id)`; time
ranges resolve to id bounds through `stats`.

## Migrations

`schema::migrate` applies forward-only steps, each in one transaction that
also bumps `meta.schema_version`. Opening a segment with a newer schema fails
with a clear error. Fixtures for every released schema version live in
`crates/peekbuddy-store/tests/fixtures/schema/` and are regenerated with:

```sh
PEEKBUDDY_WRITE_FIXTURES=1 cargo test -p peekbuddy-store --test schema -- --ignored
```

## Interner

The writer keeps a `(parent_id, name) -> node id` map (`interner::Interner`):
a hit touches no SQL and allocates nothing; a miss inserts the node and its
FTS row inside the batch transaction. After a restart the map is empty and a
miss falls back to `INSERT ... ON CONFLICT DO NOTHING` plus a lookup.
Call `Interner::clear` after a rolled-back batch.
