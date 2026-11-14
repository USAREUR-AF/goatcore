# Concurrent identity creation: measurement and follow-up

**Status:** built probe and regression coverage; concurrent acceptance measured
in the Linux smoke run. Post-resolution password checks are built but not yet
measured on the database fixture. This work is separate from the OS package refresh in
[PR #433](https://github.com/USAREUR-AF/goatcore/pull/433).
Investigation: [issue #435](https://github.com/USAREUR-AF/goatcore/issues/435).

## Why the previous result is insufficient

The F3 soak probe counted HTTP 200, but the pinned Kratos v1.3.1
`POST /admin/identities` contract returns HTTP 201 for creation and 409 for
a conflict. The driver discarded response bodies and emitted only a boolean.
Consequently, historical F3 failures cannot establish whether two identities
were accepted, one was rejected, or a request failed. In particular, 201/409
and 201/201 both produced a zero success count.

The replacement starts both requests together, keeps separate statuses and
accepted identity IDs, and uses distinct fixture passwords. It never prints
passwords or response bodies. Its classifications are:

- `unique_conflict`: one valid 201 identity response and one 409.
- `both_accepted`: two valid 201 identity responses.
- `inconclusive`: every other combination, including transport failures,
  server errors, malformed creation responses, or two conflicts.

Neither concurrent acceptance nor a rejection proves eventual convergence,
credential integrity, or successful login. Accepted identities are retained
for inspection, not automatically deleted or merged.

## Architectural boundary

[ADR 1084 D3](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md)
is accepted and keeps admin mutations on one writer. Calling two admin
endpoints deliberately goes outside that routing discipline.
[ADR 1010 D4/D6](../adr/1010-ory-on-spock-mindful-operation.md) describes
API-layer serialization and an F3 HTTP-200 expectation, but independent
application instances cannot establish cross-node uniqueness from a local
unique constraint alone.

[ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md) and
[ADR 1205](../adr/1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md)
are proposed. The [island-mode design](island-mode-multi-writer.md) places email
collisions in its unique-by-value class and proposes explicit resolution.
This measurement change does not activate that design or change database
constraints, replication, identity ownership, or login behavior.

## Verification and remaining work

Run the classifier and concurrent-submission regression suite:

```sh
python3 tests/test_ory_concurrent_identity.py
bash -n ops/ansible/roles/ory/tests/soak/run-2node-spock.sh
```

On a disposable Linux fixture, run:

```sh
docker build -t postgres-spock-test:contribs ops/postgres/spock
SOAK_DURATION=5m bash ops/ansible/roles/ory/tests/soak/run-2node-spock.sh
```

The smoke workflow now records the corrected observations and separately
requires an observed duplicate rejection. Keep this change in draft if that
gate fails; do not turn dual acceptance or an outage into a pass to merge it.
The existing wide-repset fixture is not proof of production narrow-repset
behavior.

Before claiming the underlying conflict behavior is fixed:

1. Run a same-node concurrent-create control and an independent-email control.
2. Capture the two-node response statuses and IDs with replication connected,
   then repeat with replication paused and restored in a disposable fixture.
3. Inspect both identities, their credential/identifier references, replication
   health, and exception records at both nodes after a bounded settle period.
4. Attempt login on both nodes with each fixture password. Each successful
   session must name the intended identity; an unrelated existing identity
   must remain usable.
5. Decide whether the current single-writer routing needs enforcement or the
   proposed island-mode collision policy needs implementation. Do not remove
   Kratos unique indexes or delete an accepted identity to make the test pass.
6. Encode the accepted policy as the behavioral CI assertion, with failure
   coverage for lost identities, wrong-account login, and replication failure.

The immediate CI assertion makes the old F3 expectation visible; it is not
approval of an unimplemented distributed conflict policy.

## Measured concurrent acceptance and resolution follow-up

[PR #436 smoke run](https://github.com/USAREUR-AF/goatcore/actions/runs/43113796109/job/130837526955)
measured nine concurrent-create pairs: five returned 201 from both nodes with
distinct accepted IDs, and four returned one 201 and one 409. The strict
observation gate failed; the single-node soak and probe unit tests passed.
This establishes the race in the wide-repset fixture, not eventual identity
loss or broken login.

The existing narrow-repset scenario
`ops/postgres/spock/tests/partition-harness/scenarios/kratos-c4-same-email-both-sides.sh`
already tests the ADR 1205 hub-directed resolution. It passed in
[#433's partition run](https://github.com/USAREUR-AF/goatcore/actions/runs/43079557060/job/130728188502),
including matching survivor IDs, equal identifier mappings and zero drift
across 33 replicated tables. Its previous shared fixture password could not
distinguish retained from losing credentials.

The follow-up uses different passwords, establishes that each works locally
while isolated, and checks password login at both sites after resolution.
The survivor password must return the chosen identity in an active session;
the losing password must return Kratos's invalid-credentials message, not a
network or server error. A separate pre-existing fixture identity must still
log in. Only identity IDs are printed, never session tokens.

These additional checks still require a signed follow-up and a Linux run.
They do not authorize automatic deletion of real identities or declare the
proposed island-mode policy deployed.
