# goat-logship role tests

`syntax-check.sh` runs the parse-only smoke, mirroring the sibling
`otel-collector` role, and then the GHSA-x44g checks below. End-to-end
verification (objects land off-box in S3, the host keeps only the buffer, and
the absence-alarm fires when the collector is stopped) is the Phase-1 live
re-probe on ZAGROS per ADR 1078 §6 — not a CI fixture.

GHSA-x44g (the collector ran as root with the Docker socket, metrics on every
interface):

- `render.sh` renders the compose, Vector, proxy and nftables templates with
  `render.yml` and checks them with `check_render.py`: Vector mounts no Docker
  socket and nothing under `/var/lib/docker`, runs as a non-root uid:gid with
  no capabilities and no root group, and publishes 9598 only on loopback, the
  mesh address and the bridge gateway (every interface only with
  `logship_bind_all`); only the Docker API proxy mounts the socket, it has no
  network and no TCP listener, only a unix socket in a directory Vector
  mounts read-only, and every `docker_logs` source uses that socket; with the
  container-stdout sources off there is no proxy and no socket at all. It
  also loads the guard ruleset in a private network namespace.
- `guard.sh` (with `guard-lab.sh`) builds a stand-in host in a private user +
  network namespace, with peers on `wt0`, the scraper's bridge, another
  container bridge and a LAN link, loads the rendered guard, and checks both
  Docker's DNAT path and the docker-proxy path: the mesh address only from
  the mesh and loopback, the gateway only from the scraper's bridge and
  loopback; a no-guard control run reaches the port from everywhere.
- `bind.sh` runs `tasks/bind.yml`: no IPv4 on the mesh interface stops the
  play; the address comes from the interface's facts or
  `logship_bind_address`; the scraper's bridge comes from its Docker network
  id; without that network the gateway is not bound.
- `access.sh` runs `tasks/access.yml` against scratch files as the invoking
  user: a root-only audit log is regrouped (auditd.conf `log_group`, 0640 /
  0750) and its group, not root, is added to the collector; an owner-only log
  with management off stops the play, as do a uid-0 collector user and a
  proxy user equal to the collector.
- `datadir.sh` runs `tasks/datadir.yml` on scratch dirs with planted
  symlinks: the hand-over regroups the collector's files and leaves link
  targets alone, runs only once (or again when a collector still ran as
  root), and a mock sink dir swapped for a link is not followed.
- `docker-proxy.sh` (opt-in; SKIPs without docker or the pinned nginx image
  present locally; every run uses `--pull never`) brings up the rendered
  compose's proxy service against this machine's Docker daemon and checks
  that the collector's uid/gid reaches its socket for ping, version, events,
  container list, inspect and logs; that writes (kill, create, exec, delete,
  archive) and other reads (export, archive, images, info, secrets, encoded or
  dot-segment paths) are 403; that the proxy has no network address, the
  socket is 0660 in a 0710 dir (also after a restart); and that a host user
  and other containers, even with the directory mounted, cannot connect.

The event dictionary ↔ Vector-transform contract is enforced separately and
in CI by [`ops/ci/check-logship-dictionary.sh`](../../../../ci/check-logship-dictionary.sh):
every `metadata.dictionary_id` a transform emits must map to a row in
[`ops/logging/goat-logship/dictionary.yaml`](../../../../logging/goat-logship/dictionary.yaml).

Run: `./syntax-check.sh`.
