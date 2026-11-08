# goat-zenoh-portal

Vendor self-service portal + message inspector for the Zenoh trial fabric.

A single Go binary plus an embedded static HTML/JS frontend. Postgres registry of
vendors, topics, formats, and schema references. Renders the zenohd
`access_control` config from the registry and SIGHUPs the router on change.
Inspector reads through the existing Zenoh REST plugin (`GET
http://<router>:8000/<key-expr>`) and pushes live updates via SSE.

Authoritative design lives at
[`docs/design/zenoh-trial-fabric.md`](../../docs/design/zenoh-trial-fabric.md)
§"Vendor self-service portal + inspector." The companion identity substrate
is at
`docs/design/nova-x509-identity.md`;
the community-facing onboarding packet is at
[`docs/operations/zenoh-trial-onboarding.md`](../../docs/operations/zenoh-trial-onboarding.md).

## Layout

```
ops/goat-zenoh-portal/
├── cmd/portal/main.go                # entry point + flag parsing + subcommands
├── internal/
│   ├── acl/                          # zenohd access_control config render + SIGHUP
│   ├── api/                          # HTTP handlers + auth middleware
│   ├── certmint/                     # cert-sign shell-out + tarball assembly
│   ├── config/                       # env + flag loading
│   ├── registry/                     # Postgres registry (sqlx + pgx + raw SQL per ADR 0413)
│   ├── safego/                       # panic-recovery goroutine wrapper
│   └── zenohproxy/                   # Zenoh REST proxy + SSE live tail
├── web/static/                       # vanilla-JS frontend (embed.FS)
│   ├── index.html                    # vendor home
│   ├── topics.html                   # vendor topics
│   ├── inspector.html                # topic inspector
│   ├── app.js                        # shared frontend logic + decoders
│   ├── decoders.js                   # format-aware decode (json/cbor/msgpack/protobuf/text/raw)
│   └── styles.css
├── testdata/                         # golden fixtures
├── Dockerfile                        # Chainguard glibc-dynamic runtime per ADR 0700
├── go.mod / go.sum
└── README.md
```

## Running locally

```bash
cd ops/goat-zenoh-portal

# Build
go build -o /tmp/goat-zenoh-portal ./cmd/portal

# Run with a FAKE Zenoh REST endpoint + FAKE cert-sign (good for dev)
/tmp/goat-zenoh-portal serve \
    --listen 127.0.0.1:8090 \
    --db-dsn "postgres://postgres@127.0.0.1:5432/portal_dev?sslmode=disable" \
    --zenoh-rest http://localhost:8000 \
    --zenoh-rest-fake \
    --acl-config /tmp/zenoh-config.json5 \
    --acl-storage-dir-prefix /tmp/zenoh-storage \
    --acl-reload-fake \
    --cert-sign-fake \
    --operator-api-key dev-operator-key
```

Then open `http://127.0.0.1:8090/` in a browser. Operator auth via
`Authorization: Bearer dev-operator-key` (or set the same value in the
browser's localStorage at key `portalApiKey`).

## API surface

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET`    | `/api/health`                                 | none     | Liveness probe |
| `GET`    | `/api/vendors`                                | bearer   | List vendors |
| `POST`   | `/api/vendors`                                | operator | Register a new vendor (mints cert tarball) |
| `GET`    | `/api/vendors/{vendor}/topics`                | bearer   | List topics under a vendor |
| `POST`   | `/api/vendors/{vendor}/topics`                | vendor or operator | Register a new topic |
| `GET`    | `/api/vendors/{vendor}/topics/{id}`           | bearer   | Topic details |
| `PATCH`  | `/api/vendors/{vendor}/topics/{id}`           | vendor or operator | Update format / schema ref / description |
| `DELETE` | `/api/vendors/{vendor}/topics/{id}`           | vendor or operator | Retire a topic |
| `GET`    | `/api/topics/latest?key=<key-expr>`           | bearer   | Last value (proxies Zenoh REST plugin) |
| `GET`    | `/api/topics/recent?key=<key-expr>&n=20`      | bearer   | Last N values |
| `GET`    | `/api/topics/stream?key=<key-expr>`           | bearer   | SSE live-tail; at most `PORTAL_MAX_STREAMS_PER_PRINCIPAL` (default 8) open per org/slot key, operators exempt, else 409 `stream_limit` |
| `GET`    | `/api/org-bundle/{vendor}/{token}.tar.gz`     | token    | One-time cert tarball download (returned at org mint) |
| `GET`    | `/api/vendor-bundle/{vendor}/{token}.tar.gz`  | token    | Deprecated alias of `/api/org-bundle` (ADR 1052); counted toward the Phase-4 gate |

## Coordination with Track A

This portal interacts with `zenohd` at two points; both are configurable so
the portal can be built and tested against fakes before Track A
(`nova-trust-domain-and-x509`) lands its substrate.

| Surface | Track A contract | Portal fake-mode behavior |
|---|---|---|
| `cert-sign --uri-san urn:goat-nova:vendor=<v>` | Track A extends `ops/enrollment/cmd/cert-sign/` with `--uri-san` (per `docs/design/nova-x509-identity.md` §"Substrate extension required"). Portal shells out with that flag. | `--cert-sign-fake` produces a stub cert + key + tarball (test mode only). |
| Live `config.json5` marker edits + SIGHUP via Docker API | The portal bind-mounts the goat-zenoh role's live `config.json5` r/w + the host docker socket + the per-vendor rocksdb parent dir, applies marker-based incremental edits between `BEGIN/END: per-vendor {write rules, subjects, policies, storages}` markers, and SIGHUPs the router container via `POST /containers/<name>/kill?signal=SIGHUP` over the docker socket. | `--acl-reload-fake` writes the config edit but skips the SIGHUP. |

Coordinate the exact contract via the active-work bus when both tracks are in
flight together.

## Tests

```bash
go test ./...
```

End-to-end smoke (FAKE Zenoh + FAKE cert-sign) lives at
`internal/api/smoke_test.go`.

## ADR / design references

- [ADR 0006](../../docs/adr/0006-multi-agent-human-concurrency-governance.md) — release per concurrency governance; emit bus info on first non-rehearsal cutover
- [ADR 0112](../../docs/adr/0112-tech-strategy-workload-shape-language-boundary.md) — request-shaped Go service; correct language pick
- ADR 0413 — sqlx + raw SQL, no ORM
- ADR 0700 — Chainguard runtime
- [ADR 1015](../../docs/adr/1015-code-review-discipline.md) — Tier-1 self-review at block completion
