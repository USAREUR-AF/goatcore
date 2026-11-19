# pgbackrest role tests

Parse-only smoke. Backup + restore behavior is covered by the
Wave 1.8 paired-stanza smoke harness
and exercised on every per-site bootstrap (Phase 5 in
[`site-bootstrap.yml`](../../../site-bootstrap.yml)).

Run: `./syntax-check.sh`.

`./repo-ownership-test.sh` runs `tasks/repo-ownership.yml` against a fixture
directory, locally and with no privilege: a clean repository passes, and a
root-owned file (made through a container) stops the run with its path named.
It needs `ansible-playbook` and `docker`.
