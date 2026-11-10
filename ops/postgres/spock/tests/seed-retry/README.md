# Seed Retry Regression

Run from the repository root with Docker available:

```sh
python3 -m pip install pyyaml==6.0.2 jinja2==3.1.6
python3 ops/postgres/spock/tests/seed-retry/test_seed_retry.py
```

The test starts a disposable, network-isolated PostgreSQL 16 container using
the same pinned image as the revocation-rollout suite. It reads the seed sets
and dump/restore shell from the real playbook, renders its SQL templates, and
loads the repository's complete NetBird/Kratos guard and hub-ledger SQL.
Application tables and Spock node metadata are synthetic fixtures. No live
inventory, host configuration, credentials or database is used.

It verifies the old failure against populated ledgers and an empty-ledger
positive control, revoked-key and inactive-identity backfill, founder/joiner
row equality after retry, repeated retries, all four trigger firing modes,
DELETE/UPDATE refusal after restore in origin and replica sessions, rollback
on a SQL error or missing host dump, non-seed row preservation, initial seed,
and rejection of a registered Spock node or failed node probe.

This is database and restore-shell coverage. It does not establish Ansible
SSH/delegation behavior, full application-schema compatibility, Spock
replication, or successful deployment on a live joiner. A database transaction
does not make the multi-database play atomic.
