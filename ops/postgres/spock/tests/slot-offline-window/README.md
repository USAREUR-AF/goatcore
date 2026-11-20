# slot-offline-window

How much WAL a provider's slot for an absent subscriber takes before Postgres
invalidates it, what the subscriber sees when it returns, and the re-seed that
brings it back. It is the measurement behind
[docs/design/goatbuilder-hardware-measurement.md](../../../../../docs/design/goatbuilder-hardware-measurement.md), "How long a phone may be off";
`run.sh`'s header says what it does and what it records.

```bash
docker build -t postgres-spock:offwin ops/postgres/spock
IMAGE=postgres-spock:offwin RATE_MIB=4 ops/postgres/spock/tests/slot-offline-window/run.sh
IMAGE=postgres-spock:offwin RETURN=restart ops/postgres/spock/tests/slot-offline-window/run.sh
```

A run writes about one and a half caps of WAL on the founder container, so it
takes a few minutes and a few gigabytes of disk. It is not in CI. Linux Docker
only, as every two-site test in this directory.
