# warrant runbook — changing what can reach what

**This is the one doc you need to change the mesh ACL.** Rollback is at the
bottom. Decisions live in [ADR 1129](../adr/1129-warrant-scoped-expiring-network-authorizations.md);
the design is [`warrant.md`](../design/warrant.md); you should not need either
to do the tasks below.

> **`ops/ansible/netbird_acl_matrix.yml` is GENERATED. Do not edit it.**
> It is produced from `ops/warrant/` by `ops/warrant/generate.py`, and CI
> refuses a hand-edit. If you have edited it already, jump to
> [I edited the generated file](#i-edited-the-generated-file-by-mistake).

---

## The 30-second version

| you want to | do this |
|---|---|
| Find out why A can't reach B | `goat warrant why-blocked --scope <env> --from A --to B --port N` |
| Open a port for a pair | edit `ops/warrant/rule-intents/netbird-acl.yaml`, run `ops/warrant/generate.py`, commit **both** files |
| Change a port everywhere | edit `ops/warrant/port-sets.yaml`, run the generator — its own PR |
| Grant something temporarily | add `expires:` and `owner:` to the rule intent |
| See the vocabulary | `goat warrant port-sets` |
| Give a device a group, or take one away | add a `grant:` / `absent:` entry to `ops/warrant/membership/<scope>/people.yaml` (§4A) |
| Quarantine a peer / release it | add a `hold: quarantine` entry / delete it (§4A) |
| See what the mesh would change | `goat warrant apply --membership --scope <scope> --file ops/warrant/membership/<scope>/ --mode report` |

Two files for access rules, one file per goatnet for membership. Everything below is detail on those.

---

## 1. First: it is usually not the ACL

Most "I can't reach X" reports are DNS, a membership the peer does not hold, or
a service that is not listening. Start here, because it costs seconds and needs
no credentials, no network and no VPN:

```
goat warrant why-blocked --scope zagros --from DEVELOPER --to INFRA --port 7447
```

It reads the deployment's declaration — the directory `GOAT_ACL_DECLARATION_DIR` names, else the `ops/ansible/netbird_acl_*.yml` of the checkout or overlay you run it in, else the copy compiled into the binary — and answers one of three ways.

**PERMITTED** — the ACL is not your problem:

```
ZAGROS-DEVELOPER -> ZAGROS-INFRA  tcp/7447  [scope: zagros]

PERMITTED by the declared ACL. Granted by:
  - all → infra (web/dns/fabric tcp)  (tcp 443, 53, 5353, 7447)

So this is not an ACL gap. The remaining candidates, in the order
they are usually the answer:
  1. the peer does not hold ZAGROS-INFRA (check `goat who`)
  2. nothing is listening on tcp/7447 at the far end
  3. DNS — the name resolves somewhere else, or not at all
```

**BLOCKED** — it tells you the smallest set that closes the gap and hands you
the diff to paste. See §2.

**A refusal** — if you name a group the scope does not have, it refuses the
question rather than answering "blocked", and offers near matches:

```
error: --from ZAGROS-DEVELOPERS is not a group in scope 'zagros'.
  This is a malformed question, not a blocked path.
  Did you mean: ZAGROS-BACKEND-INFRA, ZAGROS-EXTERNAL-INFRA, ZAGROS-INFRA
```

That distinction is deliberate. Answering "blocked" to a typo is how two
withdrawn findings got written on 2026-08-13.

**What it cannot tell you.** It reads the declared plane — the YAML in git. It
does not consult live group membership, the listening surface, or DNS, and it
says so on every run. It can prove a path is *not permitted*; it cannot prove a
permitted path *works*.

### A one-way path is usually correct, not a missing rule

The most misdiagnosed result is **"A reaches B, but B cannot reach A."** That is
normally the ACL working as designed, and the fix is almost never a new rule.

Two shapes produce it deliberately:

- **Capability groups.** `P2P` grants *outbound* lateral reach and nothing
  inbound ([p2p-graduation.md](p2p-graduation.md)). Measured on both goatnets,
  `INFRA` can initiate to eighteen groups and **`P2P` is not one of them**. A
  peer whose only group is `P2P` therefore reaches everything and is reached by
  nothing — including the mgmt host. **Its role group is what makes it
  reachable.**
- **Publish-only gear classes.** `SENSOR-FEED`, `SENSOR-TELEMETRY` and
  `EFFECTOR-TELEMETRY` are source-only by construction (ADR 1116 §2): a box
  that also accepts tasking holds a tasking class as well, and `INFRA → gear`
  covers seven of those. `MOON` is source-only deliberately and permanently
  (ADR 1092 — sovereign, reveal-only; nothing initiates toward it, not even the
  substrate).

- **A subordinate scope whose own `INFRA` group is empty.** This is the
  least obvious. A scope stamped under another site has its own
  `<SCOPE>-INFRA`; when that group holds **zero peers**, the scope-local `infra → gear` rule resolves
  against an empty group and permits nothing. The cross-scope substitute is
  *refused by design* — `acl_scope.py` allows only `PLATFORM-ADMIN` as a
  qualified source, with the comment *"INFRA is excluded deliberately: machines
  get function, not any-port"* (ADR 1124). So the host site's mgmt hosts
  structurally cannot reach that scope's gear, while a human on the host site's
  `PLATFORM-ADMIN` profile reaches it any-port. **The working path exists; it
  just is not the mgmt host.** Authoring `HOST:INFRA` rules to "fix" it does not
  even apply — the cutover play refuses them at Phase 1.2c.

**So check membership and scope before you write a rule.** Ask
`goat warrant why-blocked --scope <env> --from INFRA --to <group> --port 22`,
and read the peer's actual groups on the netvis identity card. Authoring
`infra → P2P` or `infra → SENSOR-FEED` to fix one unreachable box grants the
substrate inbound reach to **every** peer in that class on the goatnet — a
fleet-wide widening to solve a single membership error.

> **Consequence worth knowing, and it is not an ACL problem.** Because a
> mgmt host cannot reach a subordinate scope's gear, **guards that run on the
> mgmt host structurally cannot see those machines** — they will report clean
> about a scope they never looked at. That is a *vantage* gap, and closing it by
> loosening the ACL would be the wrong fix; the answer is where the guard runs,
> or a `PLATFORM-ADMIN`-scoped vantage.

---

## 2. Worked example — opening a port

Say an observer needs SSH to the substrate on ZAGROS. Ask first:

```
$ goat warrant why-blocked --scope zagros --from OBSERVER --to INFRA --port 22

ZAGROS-OBSERVER -> ZAGROS-INFRA  tcp/22  [scope: zagros]

BLOCKED by the declared ACL — no rule grants this pair that port.

What DOES exist between the pair:
  - all → infra (web/dns/fabric tcp)  (tcp 443, 53, 5353, 7447)
  - all → infra (dns/fabric udp)  (udp 53, 5353, 7448)
  - all → infra (icmp)  (icmp any port)

The minimal port-set that closes it: shell
  tcp 22
  owner: platform-admin
  Interactive login — tcp/22, and nothing else. Break-glass access and
  network administration. ...

The diff to put in a PR — ops/warrant/rule-intents/netbird-acl.yaml:

      - name: "observer → infra (shell)"
        description: "<why this pair needs this set — one sentence a reviewer can check>"
        sources: [OBSERVER]
        destinations: [INFRA]
        protocol: tcp
        port_set: shell
        action: accept
        bidirectional: false
```

**Step 1 — paste that block** into `ops/warrant/rule-intents/netbird-acl.yaml`,
in the section for the scope you are changing, and **write a real
`description:`**. A reviewer approves on that sentence; `<why this pair needs
this set>` left verbatim is a reviewer's cue to reject.

**Step 2 — regenerate:**

```
ops/warrant/generate.py
```

**Step 3 — check your work before pushing:**

```
python3 ops/ci/check-warrant-registry.py
```

**Step 4 — commit BOTH files.** The intent and the generated matrix travel
together; committing only one fails the gate.

**Step 5 — open the PR.** `CODEOWNERS` routes it to the owner of the path you
touched, and CI posts whether your change *widens* or *narrows* the ACL. A
widening waits for its owner. That is the gate working, not a failure.

### The one rule when editing intents

**Never write a `ports:` line.** Write `protocol:` and `port_set:`; the
generator resolves the ports. CI rejects a literal:

```
warrant: ops/warrant/rule-intents/netbird-acl.yaml:<line>: a literal `ports:`
line in the intents source. Ports live in port-sets.yaml — name the set with
`port_set:` instead. (This is the round-trip CI rejects: editing the generated
file and pasting it back.)
```

### If no set carries your port

`why-blocked` will say so. That is a **vocabulary change**: a separate,
higher-blast-radius PR against `ops/warrant/port-sets.yaml`, owned by platform
admins, because narrowing one rule affects one pair while narrowing a port-set
affects every rule using it. Check `goat warrant port-sets` first — the seven
sets were extracted from what the rules already did, and the one you want often
exists under a name you did not expect.

---

## 3. Temporary access

Add two fields to the rule intent:

```yaml
        expires: 2026-09-30
        owner: platform-admin
```

Both are **registry-only**: `generate.py` strips them, so they never reach the
applied ACL and nothing downstream sees them. What they do:

- CI fails once the date passes.
- A scheduled job (`.github/workflows/warrant-expiry.yml`, daily 07:00 UTC)
  opens a PR removing the expired rule, having first proved the removal grants
  nothing new.
- `expires:` without `owner:` is refused — an expiring grant with nobody to ask
  gets renewed by whoever is on shift when CI goes red.

If a grant is still needed when it lapses, **do not just move the date**: say in
the PR why it is still needed and who is asking. That question getting asked on
a schedule is the entire mechanism.

---

## 4. Changing a port everywhere

Editing `ops/warrant/port-sets.yaml` changes every rule that names the set —
that is the point, and it is why it is its own PR. The Zenoh TCP port existed as
eight identical literals before this; now it is one line.

```
$EDITOR ops/warrant/port-sets.yaml
ops/warrant/generate.py
python3 ops/ci/check-warrant-registry.py
```

Read the generated diff before committing. If it touches more rules than you
expected, that is the set doing its job — decide whether you meant it.

---

## 4A. Membership — which peer holds which group

On a goatnet whose `ops/warrant/membership/<scope>/_scope.yaml` says
`converge: enforce`, **warrant owns peer membership** ([ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md),
design [`mesh-change-control.md`](../design/mesh-change-control.md) §4). Nobody edits
groups in the NetBird dashboard there: the everyday dashboard is read-only, the
admin dashboard refuses group changes, and a converger on the management host
puts back anything that differs from the declaration within about five minutes.

With `converge: report` the directory is an adoption draft: the converger reports
the difference, and `goat admin members move --apply` and
`goat admin platform-admin` still write membership live. Changing `_scope.yaml`
to `converge: enforce` in a reviewed PR is the switch.

**Where things live.** Each goatnet has a directory; each file has an owner:

| File | Holds | Written by |
|---|---|---|
| `_scope.yaml` | `converge: report` or `enforce` | platform admins, by hand |
| `_baseline.generated.yaml` | each person's default groups, from their operator manifest | `python3 ops/warrant/membership.py --generate-baseline` — never by hand |
| `substrate.yaml` | management and infrastructure hosts | by hand |
| `services.yaml` | external service hosts (`exact: [<P>-EXTERNAL-INFRA]`) | `ops/scripts/external-service-finish.py` |
| `holds.yaml` | quarantine and probation | `goat admin members move --write-entry`, `lifecycle-sweep.py` |
| `people.yaml` | exceptions for people's devices (P2P, an extra role) | `goat admin members move --write-entry`, `goat admin platform-admin` |
| `orgs/<org>.yaml` | an organisation's own exceptions | that organisation's admins (a CODEOWNERS row routes review to them) |

**A device's default groups** come from its owner's operator manifest
(`ops/operators/<env>/<handle>.yml`, `netbird.auto_groups`); a phone or tablet
gets `OBSERVER`. Machines and anything not named after a person fall back to the
groups on the enrollment key that minted them. The files list only the
exceptions to those defaults:

```yaml
# people.yaml
version: 1
entries:
  - peer: peer-alice-zagros-dev-2026-09-01-laptop    # exact NetBird device name
    grant: [ZAGROS-DEV-P2P]                            # add to the default groups
    absent: [ZAGROS-DEV-OBSERVER]                      # take away, and keep away
    owner: alice
    reason: development clique (p2p-graduation.md reason 1)
    expires: 2026-12-01
```

```yaml
# services.yaml
version: 1
entries:
  - peer: example-service-01
    exact: [ZAGROS-DEV-EXTERNAL-INFRA]                 # the whole set; ignore the defaults
    owner: user369x
    reason: external service host (ADR 1178)
    expires: 2026-12-31
```

```yaml
# holds.yaml
version: 1
entries:
  - peer: <device>
    hold: quarantine                                 # ZAGROS-DEV-QUARANTINED alone
    owner: user369x
    reason: <why>
    expires: 2026-09-30                              # review date
```

**When a person's role changes,** change their operator manifest and run
`python3 ops/warrant/membership.py --generate-baseline --operators-dir <your clone>/ops/operators`
and commit the regenerated `_baseline.generated.yaml`; every one of their
devices follows, with no per-device entry. The manifests are gitignored, so CI
cannot tell whether that file is current: `membership.py --check` with the same
`--operators-dir` is how you prove it before opening the PR.

**To change membership:**

1. Edit the right file (or let a command write the entry:
   `goat admin members move ... --write-entry` does, on a governed goatnet).
2. `python3 ops/warrant/membership.py --check` — the same check CI runs.
3. Open a pull request. CI labels removing a group, adding an `absent:` or a
   hold as a narrowing, so one owner's approval is a formality; anything that
   widens reach waits for a real review (§4 of [`warrant.md`](../design/warrant.md)).
4. After merge, deploy the directory to the goatnet, from `ops/ansible`:
   `../scripts/ansible-preflight.sh -i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini warrant-membership.yml -e target=<site>_hosts`.
   The converger applies it on its next pass.

**Rules CI enforces:** one entry per device across the whole directory; group names carry the goatnet's prefix
and exist in the taxonomy; `EXTERNAL-INFRA` only as `exact: [<P>-EXTERNAL-INFRA]`
(a service host holds it alone, #1286); `PLATFORM-ADMIN` only on a person's device;
an `expires:` on every hold and on every grant of `EXTERNAL-INFRA`,
`PLATFORM-ADMIN`, `BREAKGLASS` or `P2P`.

**Quarantine:** add the `hold:` entry (it replaces any entry the peer had);
release by reverting that commit (`git revert <sha>` in a PR), which puts the
earlier entry back — or deletes the hold if there was none. The peer gets back
exactly what its key and that entry say, so there is nothing to remember.
For "off the mesh this second", `goat admin peer cut` is still the tool.

**What the converger will not do:** touch `All` or groups of another prefix; change
a peer it has no key baseline for and no `exact`/`hold` entry for (reported as
`unknown`); make more than `--max-changes` (10) changes in one run — it makes none
and fails instead.

**Quarantine guard exits:** exit 2 means enforce exceeded the declaration's
report-only ceiling, exit 3 means the max-changes limit stopped the whole run,
and exit 4 means one or more person quarantine rows were refused by the
privileged-account gate after other eligible rows converged. Report-only runs
stay exit 0 and expose refused rows as `blocked` in the report and in
`goat_warrant_membership_privileged_quarantine_blocked`. For a proven
uniquely owned privileged person, use ADR 1091 severance and credential
rotation. An ambiguous join proceeds only when every possible owner is proven
ordinary; if one possible owner is privileged, the refusal stays unproven so it
does not direct the operator to sever the wrong person. When evaluating ownership,
the candidate grammar accepts both the current person-peer form and the legacy
pre-v1.1.0 `peer-H-H-<scope>-<date>...` form so older peers remain reachable by
security controls; only an exact repeated `H` is accepted, not an arbitrary
multi-token org. For a missing owner,
ensure the person's manifest declares the scope in `netbird.sites`, regenerate
the membership baseline, and deploy it. Regeneration does not resolve a
handle-prefix collision or a shared email hash: deduplicate conflicting
manifests, or rename a colliding handle and regenerate. Machines do not require
an Ory owner join.

**Emergencies.** On a governed goatnet membership is never edited by hand: the
admin dashboard (`admin-dashboard.<site>`) refuses group writes there (403 from
`mgmt-admin.<site>`), and anything that got through another way is put back on
the next pass. Use the path that matches the situation:

| Situation | Do this |
|---|---|
| Access must come off now | Write the entry (removal, `absent:` or `hold:`). CI proves it a narrowing, so one owner approves it at once; then deploy the file |
| A device must be off the mesh this second | `goat admin peer cut <peer>`, then write the `hold:` entry so the record exists |
| The converger or the declaration is doing the wrong thing | Pause the converger on the management host while the fix is merged: |

```
sudo -u root goat warrant pause --scope <site> --for 1h --reason "<what and why>"
```

While the pause is in force every converger run on that scope is report-only
(`goat_warrant_membership_paused{scope}` = 1). It expires on its own (24h at
most), and the pause and its reason are recorded as an operator action. To end
it early, once the fix is merged:

```
sudo -u root goat warrant resume --scope <site>
```

Do not stop `warrant-membership.timer`: that is unrecorded and never ends on its
own. The admin address is mainly for what warrant does not cover.

---

## 5. Seeing it work without touching anything

```
ops/warrant/demo/run.sh
```

A complete example policy that belongs to nobody
([`ops/warrant/demo/`](../../ops/warrant/demo/README.md)) — nine rules, five
port-sets, no network. It generates an ACL, shows that the source carries zero
literal ports, and moves every affected rule from a single edit. Useful for
learning the shape, and for showing someone else what this is.

---

## Rollback

**A rule change you want to undo:** `git revert` the commit and redeploy. The
decision and the artifact are the same object, so there is no second store to
reconcile.

**Removing warrant entirely:**

```
rm -rf ops/warrant ops/ci/check-warrant-*.py .github/workflows/warrant-*.yml
```

and revert the `warrant` block in `.github/CODEOWNERS`.
`ops/ansible/netbird_acl_matrix.yml` goes back to being hand-maintained with no
other change — **it is byte-identical to what it was before warrant existed**,
which is the property the cutover was built around.

---

## I edited the generated file by mistake

You will see:

```
warrant registry gate: FAIL

  - BYTE-IDENTICAL: the registry does not reproduce ops/ansible/netbird_acl_matrix.yml.
  The generated matrix is the deployed one — a difference here means the
  registry and the fleet disagree about what is permitted.
  Regenerate with: ops/warrant/generate.py
```

followed by a diff of *on disk* versus *what the registry says*. Move your
change into `ops/warrant/rule-intents/netbird-acl.yaml` as a `port_set:`
reference, then run `ops/warrant/generate.py`. If you would rather start over:
`git checkout ops/ansible/netbird_acl_matrix.yml`.

---

## What this does not do

- **It does not observe traffic.** It can tell you nothing is listening on a
  granted port; it can never tell you a granted port went unused.
- **It is file-level.** Every check here can pass while a goatnet runs flat on
  NetBird's Default All→All policy, because nothing here reads a live mesh.
  Live posture is the cutover play's `mode=propose` report.
- **Narrowing auto-approval is proven but not wired.** CI *proves* a change
  grants nothing new; the approving click still needs a human, because GitHub
  will not let a workflow approve its own PR.

## See also

- `mesh-acl.md` — what the groups are and what they mean
- [`netbird-acl-cutover-runbook.md`](netbird-acl-cutover-runbook.md) — applying the ACL to a live goatnet
- `goat acl test` — running the ACL assertions
