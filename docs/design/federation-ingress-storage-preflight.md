# Federation ingress storage preflight

## Status

**Built:** an edge-role preflight for inspectable local core configurations,
structured storage-scope inspection using the existing endstate JSON5 parser,
and regression tests for the retained-history upgrade path. **Not built:** remote
core observation, automatic data migration or an end-to-end delivery guarantee.
No deployment or live acceptance is claimed by this change.

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

The edge role reads `/etc/zenoh/config.json5` from the configured local core
container using an argument-vector command. Configuration reads and intermediate
facts are hidden from logs because a core config can contain credentials. The
controller-side filter reuses `json5_lite` from the endstate observer and inspects
only `plugins.storage_manager.storages`, so ACL strings and comments cannot
satisfy the check. No additional parser dependency or network call is introduced.

A readable configuration with missing scopes or malformed storage data refuses
with the missing scope names. The check does not infer success from silence.
When the core cannot be read locally, including separate core/edge hosts, it
explicitly reports SKIP and requires inspection through the core's own approved
workflow. The existing `goat_zenoh_edge_ingress_scope_check` switch can deliberately
disable this supplementary inspection; that also reports SKIP and makes no probe.
Neither case is a passing acceptance result.

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
real core renders in both identity models, explicit skips and suppressed config
secrets. The integration cases execute the actual Ansible tasks with a fake
Docker command and temporary files; no real core or edge is contacted. The
normal Python CI suite discovers these tests.

The render regression additionally retains the contributor's hyphenated sovereign
case in both identity models while keeping #235's history-preservation checks.
