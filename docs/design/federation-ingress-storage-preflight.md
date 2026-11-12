# Federation ingress storage preflight

## Status

**Built:** an edge-role preflight that reads the core configuration on the edge
host or, when the core router runs on a separate host, on the one host of the
inventory group `goat_zenoh_edge_core_group` (`<partner>_core`); structured
storage-scope inspection using the existing endstate JSON5 parser; and regression
tests for the retained-history upgrade path and the separate-host shape.
**Measured:** by `tests/test_zenoh_ingress_scope.py`, which runs the role's tasks
against a two-host inventory with a fake Docker command. The
separate-host scenario on the federation test environment is not yet measured
with this change. **Not built:** automatic data migration or an end-to-end
delivery guarantee. No deployment or live acceptance is claimed by this change.

## Required declarations and historical stores

A core storage declares a subscriber on its key expression. An edge granting
`from/xaa/**` cannot narrow a core's broader `from/**` declaration on its behalf.
A matching scoped store supplies the declaration the edge admits. The presence of
a broad historical store does not invalidate that matching scoped declaration.

PR #235 adds per-partner stores while preserving the legacy store and existing
queryable history. This preflight accepts that layout. It requires one exact
matching storage key expression for every admitted ingress scope; for per-lane
edge grants, that means matching per-lane stores. It deliberately uses exact
scope comparison rather than inventing a second implementation of Zenoh key
expression containment. Custom layouts requiring a different equivalence need
explicit operator review.

## Observation and verdicts

The edge role reads the core router's configuration from inside its container
using an argument-vector command (`docker exec <core> cat <path>`). The path is
the one the container was started with (`docker inspect`, `.Args`): `-c <path>`,
`--config <path>`, `-c=<path>`, `-c<path>` (only when the path begins with `/` or `.`) or `--config=<path>`, never the value of another option (`-l`/`--listen`, `-e`/`--connect`, `-i`/`--id`, `--cfg`), so a core run by
hand with `-c /etc/goat-core/config.json5` is read where it reads
(`tasks/core-config-path.yml`, filter `zenoh_core_config_path`). Only a router whose
arguments name no configuration is read at `goat_zenoh_edge_core_config_path`, or at
the goat-zenoh role's `/etc/zenoh/config.json5` when that is unset. Nothing is
guessed: arguments that cannot be read, a configuration argument with no usable
path, two different paths, or an override that differs from the path the arguments
name each refuse the check and say why (an override that differs prints both
paths). Measured: the first separate-host
version of this check assumed `/etc/zenoh/config.json5` and refused every such
hand-run core on the federation test environment as unreadable. The rule lives in
`tasks/core-config-path.yml`, and the role's key-storage comparison
(`tasks/key-storage-check.yml`, the key-bundle grant against the core's `keys/`
stores, on the edge host's own core only) reads the configuration by the same rule.
Before, that comparison read `/etc/zenoh/config.json5` alone and reported a hand-run
core's `keys/**` store as "could not run" instead of refusing it. Status: built,
measured by `tests/test_zenoh_edge_key_storage.py`. It looks for the core router on the edge host
first (`tasks/discover-core.yml`: `goat_zenoh_edge_core_container`, else the core
role label or name). When none is there, it looks on the core host: the one host
of inventory group `goat_zenoh_edge_core_group`, by default
`<goat_zenoh_edge_partner_id>_core`, the group the driver deploys the outbound
republisher to. Both the discovery and the read on that host are delegated to it,
over its own inventory connection; the play's `--limit` to the edge group does not
prevent that. Configuration reads and intermediate
facts are hidden from logs because a core config can contain credentials. The
controller-side filter reuses `json5_lite` from the endstate observer and inspects
only `plugins.storage_manager.storages`, so ACL strings and comments cannot
satisfy the check. No additional parser dependency or network call is introduced.

A readable configuration with missing scopes or malformed storage data refuses
with the missing scope names. The check does not infer success from silence.
A check that cannot run fails the play: no core router on the edge host and no
single core host in the group (none, several, or only the edge host itself), no
core router on the core host, or a configuration that cannot be read. Each names
the reason and the host; an unreadable configuration also names the path, the
read's exit status and its error text, never the configuration's content. Before this, every one of those reported SKIP and let the
play exit 0, so an edge on a separate host from its core admitted ingress scopes
with only a broad `from/**` store behind them. The existing
`goat_zenoh_edge_ingress_scope_check` switch can deliberately disable this
inspection; that reports SKIP, makes no probe, and is not a passing acceptance
result.

## Data lifecycle and deployment boundaries

No core storage definition, data path, ACL or trust anchor changes here. Keeping
the legacy store preserves historical samples; it can also retain copies of a
partner's data alongside the new per-partner directory. Termination and deletion
procedures must account for those historical copies without deleting another
partner's data. The orphan-store policy remains operator-controlled.

A passing preflight proves configured scope coverage only. Fresh delivery,
persistence after diagnostic subscribers stop, refusal controls and bilateral
acceptance remain separate checks. The federation e2e's hand-authored test core
is not a substitute for testing the actual role-rendered configuration.

## Verification

`python3 -m unittest discover -s tests -p test_zenoh_ingress_scope.py` covers the
legacy-only refusal, scoped-only success, legacy-plus-scoped success, missing
partner/lane scopes, JSON/JSON5 syntax, comment and ACL decoys, malformed data,
real core renders in both identity models, refusals when the check cannot run,
the explicit disable switch and suppressed config secrets. The `CoreOnAnotherHost`
cases put the edge and the core router on separate inventory hosts and limit the
play to the edge group, as phase 12 does: a `from/**`-only store there is refused,
a matching store passes, and a missing, ambiguous or unreadable core fails. The
`CoreOverSsh` cases reach the core host through a real, unprivileged `sshd` on
loopback, as a hand-run core and as a role-deployed core; they are skipped where
`sshd` is not installed. Not covered by a test: `become` on the core host, since
the tests cannot use sudo. The integration cases execute the actual Ansible tasks with a fake
Docker command and temporary files; no real core or edge is contacted. The
normal Python CI suite discovers these tests.

The render regression additionally retains the contributor's hyphenated sovereign
case in both identity models while keeping #235's history-preservation checks.
