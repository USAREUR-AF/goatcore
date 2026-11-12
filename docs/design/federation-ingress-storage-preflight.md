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

The edge role reads `/etc/zenoh/config.json5` from the core router container
using an argument-vector command. It looks for the core router on the edge host
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
the reason and the host. Before this, every one of those reported SKIP and let the
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
a matching store passes, and a missing, ambiguous or unreadable core fails. The integration cases execute the actual Ansible tasks with a fake
Docker command and temporary files; no real core or edge is contacted. The
normal Python CI suite discovers these tests.

The render regression additionally retains the contributor's hyphenated sovereign
case in both identity models while keeping #235's history-preservation checks.
