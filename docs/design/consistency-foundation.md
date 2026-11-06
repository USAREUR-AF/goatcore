# Consistency foundation — one typed intent, one comparison, one console

| Field | Value |
|---|---|
| Status | Built. IntentV1 (§3), the grading library in Python and Rust against one shared corpus (§4), the ConsistencyV1 record, publisher and rollup (§5), the consistency console (§7) and its cross-goatnet aggregator mode (§7.8) are in this repository. The config render-hash recorder (§5.2a) ships default-off. The site-local intent replica (ADR 1109 FM-2) is designed, not built |
| Scope | Every governed goatnet. Repo-side intent generation, per-node convergence reporting, the comparison that joins them, and the surfaces that render it |
| Anchor ADR | [ADR 1110](../adr/1110-typed-consistency-foundation-and-console.md) |
| Amends / concretizes | [ADR 1036](../adr/1036-environment-health-surface.md) (adds the `consistency` domain through its `oneof` plug-in path) · [ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md) FM-1 · `environment-parity.md` §5 (adds one grade to its grammar, per that section's amendment rule) |
| Related | [`fleet-management.md`](fleet-management.md) · [`universal-deploy-rollback.md`](universal-deploy-rollback.md) ([ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md)) · [`environment-health-surface.md`](environment-health-surface.md) · [`name-fallback-resolver.md`](name-fallback-resolver.md) ([ADR 1082](../adr/1082-name-fallback-resolver-signed-snapshots.md)) · [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md) · [`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md) · [`release-graduation.md`](release-graduation.md) · [ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md) |

> **60-second summary:** goat can answer "what should each goatnet run" (the fleet pin table) and "what did each host actually deploy" (the rollback ledger). This design makes the *join* of those two answers a foundation rather than something every consumer re-derives. **IntentV1** is one schema-validated, signed, generation-identified artifact per goatnet that every consumer reads; YAML stays the human authoring surface. **One grading library** defines the six grades and their remediation pointers exactly once, and both languages that execute it are conformance-tested against one golden corpus. **ConsistencyV1** is the per-node convergence record, carried as a `oneof` arm on the existing `EnvHealthV1` envelope rather than as a new surface. **The consistency console** is a mesh-internal, read-only projection with zero private state that renders the comparison as a rapid-assessment grid where every non-matching row carries a copy-pasteable remediation pointer. The rule underneath all four: **a drift verdict's remediation is always to re-converge, regenerate, roll back or declare — never a bespoke mutation**, because drift is fixed by re-running, not by hand.

---

## 1. Why this exists

The environment-parity model is stated in one paragraph:

> Consistency management here is **one data model, not a family of tools**. Two fact kinds exist, and only two — **intent** (a versioned generation, derived deterministically, identified by the git commit that produced it) and the **convergence record** (one self-published fact per node, sourced only from that node's deploy ledger). Everything an operator ever sees is the **one comparison function** applied to that pair… A projection may choose which records it can see and how it renders; it may not introduce its own collector, its own fact shape, or its own comparison semantics.

A contract like that needs a mechanism behind it. Three things stand between the model as written and the model as a foundation:

1. **Intent needs an artifact.** A pin table (`ops/pins/fleet-pins.yaml`) and a resolver are not an emitted object that says "this is generation G of goatnet X's intent." Without one, a consumer must re-run the resolver against a checkout, so the intent it sees depends on which checkout it ran against. That does not work for a Rust meter on a management host, a node fetching intent over a constrained link, or a browser.
2. **The comparison semantics must live in a library, not a renderer.** Resolution precedence (member `group_vars` → `group_vars/all.yml` → role default, with multi-role defaults refusing to resolve) and grading (`unreadable` outranks `unset` outranks value comparison, so nothing reaches `match` on a value nobody could read) are load-bearing. If they live inside one script, the second consumer makes a copy.
3. **The live half needs a shape.** The on-host `/var/lib/goat/<component>/deploy-history.jsonl` ledger is written by every deploying role and is the best-maintained fleet fact goat has. Something has to read it for consistency.

The build order is binding: the console comes after the foundations, because a console built on re-derived semantics would be one more place the comparison lives, not the first place it is visible.

**What this doc does not re-decide.** Convergence is pulled (ADR 1109 D2). There is one convergence engine, Ansible (D3). No central consistency authority is permitted (environment-parity §8). This doc specifies the data and the comparison those decisions presuppose.

---

## 2. The two fact kinds, made typed

| | Intent | Convergence record |
|---|---|---|
| Question | What is goatnet X supposed to run? | What is node N running, and against which intent? |
| Scope | one per goatnet per generation | one per node, continuously refreshed |
| Source | the committed tree — `fleet-pins.yaml` + `parity-capabilities.yaml` + governed `group_vars` renders | the node's own `deploy-history.jsonl` tips + its applied-intent stamp. Nothing else may claim to know what runs |
| Artifact | `IntentV1` — canonical JSON, signed, committed (§3) | `ConsistencyReportV1` on the bus, rolled into `EnvHealthV1{domain:"consistency"}` (§5) |
| Authority | derived; a generator owns it, no human edits it | self-published; the node is the only author |
| Where compared | nowhere — intent is never graded against itself | the grading library, at read time, never stored (§4) |

Two properties make this a foundation rather than two more files:

- **Every consumer reads the artifact, never the inputs.** A human edits `fleet-pins.yaml`; no consumer other than the generator parses it. The *resolution* of intent becomes a rendered copy, and rendered copies do not drift.
- **The comparison never appears twice in source form.** The grammar is a committed data file with a shared golden corpus (§4), so the two languages that must execute it (Python repo-side, Rust on the fabric) are tested against the same expectations rather than trusted to agree.

---

## 3. IntentV1 — the typed intent artifact

### 3.1 Shape

One document per goatnet, at `ops/consistency/intent/<goatnet>/intent.json`, generated, committed and drift-gated. It is a *resolved* view: every value is already resolved per member, with its resolution state attached.

```json
{
  "format": "goat-intent-v1",
  "goatnet": "zagros",
  "members": ["zagros_hosts", "zagros_b_hosts"],
  "serial": 7,
  "generated_at": "…",
  "ttl_seconds": 2592000,
  "ca_id": "<the goatnet's offline CA id>",
  "content_hash": "sha256:…",
  "sources": {
    "ops/pins/fleet-pins.yaml": "sha256:…",
    "ops/pins/parity-capabilities.yaml": "sha256:…"
  },
  "components": [
    { "name": "goat-zenoh-portal", "var": "portal_image", "artifact_type": "image",
      "members": {
        "zagros_hosts":   { "state": "resolved", "value": "ghcr.io/{{ ghcr_org }}/goat-zenoh-portal@sha256:…" },
        "zagros_b_hosts": { "state": "absent",   "declared": { "reason": "portal is single-homed on the primary host" } }
      } }
  ],
  "capabilities": [
    { "var": "rosenpass_enabled", "why": "intra-mesh PQC…",
      "members": {
        "zagros_hosts":   { "state": "resolved", "value": "true",  "source": "group_vars/zagros_hosts/vars.yml" },
        "zagros_b_hosts": { "state": "resolved", "value": "false", "source": "group_vars/all.yml" }
      } }
  ],
  "node_shape": [ … ],
  "retired":    [ … ],
  "ungoverned": [ { "member": "zagros_hosts", "var": "…_image", "allowlisted": true } ]
}
```

Four content rules:

- **Resolution state travels with every value.** `resolved` / `unset` / `unreadable` / `absent` stay apart: `unset` means there was nothing to read, `unreadable` means it was read and could not be evaluated. A consumer holding only the artifact cannot recompute them, so the artifact carries them.
- **Unresolvable-by-design values stay unresolved and say so.** `{{ ghcr_org }}` is literal in the artifact: the same committed string resolves to a different registry org depending on which checkout deploys.
- **The ungoverned tail is named, not omitted.** An image the artifact stays silent about is exactly the drift it exists to prevent.
- **No secrets, ever.** Intent carries pins, capability booleans, node-shape keys and declared deviations. Anything resolved from `lookup('env', …)` renders `unreadable` with its source named, never its value. The artifact is committed and served.

### 3.2 Schema technology — JSON on disk, protobuf on the bus

**IntentV1 is canonical JSON validated by a committed JSON Schema (`ops/consistency/schema/intent-v1.schema.json`); ConsistencyV1 is protobuf, because it rides the bus.** JSON on disk and in the PR; protobuf on the wire; the bus never carries intent, only a reference to it (§5.1). The reasons:

1. **The signature is over file bytes.** The signing pattern reused here (ADR 1109 P14, [`name-fallback-resolver.md`](name-fallback-resolver.md) §3.1) signs SHA-256 over the exact bytes of a JSON document. That needs a byte-stable text artifact, and reusing the signer means no new key infrastructure.
2. **It is reviewed as a diff.** A pin bump is a one-line reviewable diff that cannot miss an environment. A binary artifact is not reviewable.
3. **Its producers and consumers are not on the bus.** The generator is Python, the CI gate is a shell step, the console reads it from disk, and a node fetches it over HTTP from `get.<site>`. Protobuf codegen buys nothing there.
4. **Precedent exists both ways.** Committed JSON with a committed JSON Schema: `ops/devices/schema/device-manifest.schema.json`, `ops/operators/schema/operator-manifest.schema.json`. Protobuf: every `EnvHealthV1` domain.

*Rejected: YAML for the artifact.* YAML is right for authoring (comments carry the `reason:` and `why:` text) and wrong for a signed generated object: one document has more than one valid serialization, which a byte-exact signature cannot tolerate.

### 3.3 Generation identity

A generation is identified by the commit that produced it — but a committed file cannot contain the hash of the commit that contains it. So:

- **The comparison key is `content_hash`** — SHA-256 over the canonical body (everything except `serial`, `generated_at` and `content_hash` itself). It is a deterministic function of the tree, computable by both sides.
- **`serial` is the human-facing monotonic name** of a generation, bumped by the generator **only when the body changes**. `generated_at` moves only with `serial`. This keeps `--check` stable: regeneration with no input change is a no-op.
- **The git commit is provenance, stamped by the actuator.** The Ansible run that applies intent records the deploying checkout's `git rev-parse HEAD` in the node's applied-intent stamp (§5.2). The commit is the name people use; the content hash is what a comparison uses.

**Content-hash comparison is a correctness property.** Two commits routinely produce identical intent (a doc-only commit, an unrelated role change). Comparing on commit would grade every node `SKEW` after every merge — a false red, and false reds are how a health surface gets ignored ([`fabric-health-surface.md`](fabric-health-surface.md) §1a). Comparing on content hash grades a node diverged only when what it should run actually changed.

### 3.3a Generation retention

§4.2 separates two grades by their relationship to history: `SKEW` is an undeclared difference where every observed value is accounted for by *some* generation in the tree, and `unaccountable` is a ref *no* generation explains. The readers that must compute this — the rollup meter, the console, a node fetching from a replica — each hold exactly one generation and no git history. With one generation, every non-matching observation looks the same.

So `IntentV1` carries a bounded history tail of the generations it succeeds — the identity triple only, never the bodies:

```json
"history": [
  { "serial": 6, "content_hash": "sha256:…", "generated_at": "…" },
  { "serial": 5, "content_hash": "sha256:…", "generated_at": "…" }
]
```

The generator already reads the committed artifact to decide whether to bump `serial`, so it knows the prior generation. A reader holding one artifact then answers both questions from it alone: a hash present in `history` is behind by a known count and age (`SKEW`, with intent age populated); a hash present nowhere is `unaccountable`. Depth is a declared constant (`HISTORY_DEPTH`, schema `maxItems: 10`); a node so far behind that it falls off the tail is not accountable to anything the reader can see, which is the correct answer.

**The tail is evidence for grading, never a target for converging.** It does not make any older generation targetable — no per-site generation pin. That would be a second authority and would make "converged" ambiguous.

`history` sits inside the canonical body, so `content_hash` and the signature cover it. As built: the schema field is optional; the generator inherits the tail in `goat_consistency.intent.render()`; the reader-side computation is `goat_consistency.grading.account_for()`, which returns `resolved` / `behind` / `unaccountable` from one artifact and one observed hash; `ops/consistency/testdata/cases/accountability.yaml` pins both grades and the off-tail case from a reader's inputs only. A node's `behind` cell lifts to `SKEW` on its own: three nodes agreeing with each other on a superseded generation are not `match`, because the reference is the artifact, not each other.

### 3.4 Signing

`intent.json.sig` is detached: base64 of an ASN.1 DER ECDSA-Sig-Value (P-256) over SHA-256 of the exact file bytes, signed by the per-goatnet offline CA named by the site's `ca_id` group_var. It reuses the name-snapshot signer machinery (`ops/enrollment/ca/age_ca.go` `AgeSigner`) and the verification path goat-cli already embeds. **No new key infrastructure.** Verification is fail-closed at every reader (ADR 1109 P14): an unverifiable intent is refused, never rendered as fact.

- **The signer** is `ops/enrollment/cmd/intent-sign`, the name-snapshot signer's sibling: same `AgeSigner`, same audit log, same base64(DER)+newline encoding, same cross-check of the signer's public key against the committed SPKI at `ops/enrollment/public-keys/<ca_id>.pem`. It is a separate command because the two artifacts carry different formats and identity fields. The generator does not sign; it prints the re-sign instruction when the body moves. Regeneration is mechanical; signing is a person holding the goatnet's CA age key; CI refuses the gap between them.
- **The trust root is the committed SPKI set**, resolved by the artifact's `ca_id` and never taken on the artifact's word. Repo-side readers resolve `ops/enrollment/public-keys/` directly; the deployed rollup gets the same set placed beside the artifact by the `consistency-health` role (`GCH_PUBKEY_DIR`). The role's copy step is therefore an untrusted transport by construction, which is what lets a site-local replica or a courier carry the artifact without carrying authority.
- **Verify-at-read is in every consumer, in both languages.** Python: `goat_consistency.signing.verify_file` (openssl, the same mechanism as `ops/ci/check-name-snapshot-sig.sh`) behind `intent.load()`, the `--check` gate, the applied-intent stamp (`intent_verified` means *signature valid and content_hash self-consistent*) and the stage-1 renderer. Rust: `goat_consistency::verify` behind the rollup meter's `load_intent`, re-run on every beat over the same bytes it parses. A context that cannot verify (no openssl) reports `unavailable` and refuses.
- **The CI gate** (the `fleet-pins` job) fails on a missing, stale or forged signature, so an intent-affecting PR needs a re-sign before merge.

Three integrity layers: the **signature** proves who produced the bytes, **content_hash self-consistency** proves the body is what it claims, and the **CI regenerate-diff** proves the tree still agrees. Readers check the first two at every read; CI checks all three.

### 3.5 Emission — committed and CI-gated, not a build artifact

The artifact is committed to the tree and `--check`-gated, like `pins.yml`, `mesh-hosts` and `name-snapshot.json`. It is not a per-commit CI artifact.

- *A CI-only artifact drifts silently in one direction:* nothing in a PR shows a reviewer that a table edit changed a resolved value for a member they were not thinking about.
- *A committed artifact drifts silently in the other direction:* it goes stale against its inputs. `--check` closes that — regenerate, diff, fail.
- The tie-breaker: **a node must be able to fetch intent without CI**, at a site whose WAN is down ([ADR 0206](../adr/0206-island-mode-by-construction.md)). A committed, signed file has the same offline story name snapshots have: served from `get.<site>`, courier-safe because the signature makes couriers untrusted.

Every intent-affecting PR carries a generated-file hunk, the same cost `pins.yml` already imposes.

### 3.6 The authoring/consumed split

**Authoring surface (human-edited):** `ops/pins/fleet-pins.yaml`, `ops/pins/parity-capabilities.yaml`, `ops/ansible/group_vars/*/`, and `ops/pins/lite-pins.yaml` — the sidecar table a goatlite is declared in ([goatlite.md](goatlite.md)): only `goatnets:` rows and `components.<name>.deviations.<member>` blocks, merged by both loaders; a goatnet declared there lists both tables in its intent `sources`. Comments, `reason:`, `why:` and `until:` live here, next to the value, where review happens.

**Consumed surface (generated, machine-read):** `intent.json` (+ `.sig`).

| Consumer | Reads |
|---|---|
| `ops/pins/report.py` (stage-1 renderer) | the tree through the library's resolver, graded through the library (§4.5) |
| The `consistency` rollup meter (§5) | `intent.json` from disk on the management host, verified at read |
| The consistency console (§7) | `intent.json` from disk + records from the bus; computes nothing else |
| `goat health --domain consistency` | the rolled envelope only |
| A node's pull loop (ADR 1109 FM-2, designed, not built) | `intent.json` + `.sig` from a site-local replica |

`ops/pins/generate-pins.py` renders `pins.yml` into `group_vars` and `intent.json` from the same resolution pass — one resolver, two renderings, the pattern `generate-mesh-hosts.py` uses for the zone file and the name snapshot.

`report.py` reaches the tree only through the same resolver the artifact is rendered from, rather than reading the artifact, because its human-readable cells print raw source detail the artifact normalises away. The rule holds either way: no consumer other than the generator parses the authoring YAML.

### 3.7 What IntentV1 is not

- **Not a superset of Ansible variables.** It carries governed pins, curated capability rows, node-shape rows, retired-key assertions and the named ungoverned tail. Growing it toward "all config" is the mega-unification non-goal.
- **Not per-host.** It is per-goatnet with per-member columns. A node reads its member's column.
- **Not a schedule.** It says what should be true, never when a node must converge.

---

## 4. The grading library — one comparison, one grammar, two conformant executions

### 4.1 Layout

```
ops/consistency/
  grammar.yaml                     # SoT: the grades, their precedence, the remediation kinds
  schema/intent-v1.schema.json     # IntentV1 (§3)
  testdata/                        # the shared golden corpus — the conformance contract
  python/goat_consistency/         # the Python implementation (importable package)
  rust/                            # the Rust implementation, the meter and the console
  intent/<goatnet>/intent.json     # the generated artifacts (+ .sig)
```

The Python package is importable, so resolution is library code rather than logic trapped in a hyphenated CLI. `report.py` and `generate-pins.py` are thin entry points over it.

**The library is the only place comparison semantics live.** A projection may choose what it can see and how it renders; it may not grade. A surface that computes its own verdict is a second model, and the first divergence between them is silent.

### 4.2 The grammar, versioned once

The grades carry identical meaning in the stage-1 output, the live join, the `ConsistencyV1` envelope and every render. A stage may add a grade only by amending the grammar.

| Grade | Meaning |
|---|---|
| `match` | Every member resolves to the same value, and every reporting node reports that value. |
| `declared` | Members or nodes differ, and a `reason:` in the authoring surface says why. |
| `SKEW` | An undeclared difference, where every observed value is accounted for by *some* generation in the tree. The node is behind, or two nodes disagree. |
| `unreadable` | A value was read and could not be evaluated (templated, defaulted by several roles, unverifiable signature on a record). **Never `match`.** |
| `indeterminate` | Nothing to read — a key with no entry and no default, or a node that has not reported. **Never `match`.** |
| `unaccountable` | A node reports a ref, hash or render that no generation in the tree explains. Not behind — unexplained. |

**Why `unaccountable` is its own grade.** It demands a different action from `SKEW`. A node one bump behind is fixed by re-running its refresh play (§6). A node running an image no committed generation ever named is a runtime mutation, a hand-deploy, or a ledger that is wrong — and re-running the play would erase the evidence before anyone looked at it. It is also the observable the persistent-state rule (no out-of-SoT state) needs in order to be checked at all.

`grammar.yaml` is the SoT for the six grades, their precedence (`unreadable` > `indeterminate` > `unaccountable` > `SKEW` > `declared` > `match` — worst wins for a row, and the two ungrounded states outrank everything), the closed set of remediation kinds (§6), and the grade → env-health colour mapping (§5.3). The pattern is `ops/identity/grammar.yaml` ([ADR 1067](../adr/1067-identity-grammar-generated-sot.md)): a committed vocabulary with a generator and a CI reproduction gate.

### 4.3 One grammar, two executions

The grammar executes in Python repo-side (the renderer, CI gates, the generator) and in Rust on the fabric (the rollup meter computes `overall_color` to publish an `EnvHealthV1`, and the console renders live records — both are long-running processes holding a Zenoh mTLS session, which is Rust by [ADR 0112](../adr/0112-tech-strategy-workload-shape-language-boundary.md)).

**The grammar is defined once as data; an implementation is permitted only if it passes the shared golden corpus in CI, and no implementation may define a grade.** `ops/consistency/testdata/` holds input/expected pairs — an intent generation, a set of records, and the exact graded output including remediation pointers. Both implementations run it. A grade whose behaviour no corpus case pins is not part of the grammar, and every grade change is a corpus diff a reviewer can read.

### 4.4 The library's output

One shape, whatever the caller:

```
GradedRow {
  subject       : component | capability | node_shape | retired | ungoverned
  name          : "goat-zenoh-portal" | "rosenpass_enabled" | …
  scope         : goatnet, and the member/node columns
  cells         : per member/node — value, state, source, and (for live rows) record age
  grade         : match | declared | SKEW | unaccountable | unreadable | indeterminate
  why           : one sentence, generated, naming what differs and where
  remediation   : the pointer (§6) — required for every grade except match and declared
  grounding     : n (members compared, nodes reporting), window, vantage
}
```

`grounding` is required by ADR 1109 P15: every fleet reading carries its `n`, window and vantage, so "100% compliant" can never mean "0 nodes reported". A `GradedRow` with no reporting nodes grades `indeterminate` with `n=0`, and no renderer may present it otherwise.

### 4.5 First consumer

`report.py` is the library's first caller and has no semantics of its own. Its acceptance when it moved onto the library was that its output regenerated byte-identically: a semantics extraction that changes output is either a bug or an undeclared decision.

---

## 5. ConsistencyV1 — the convergence record on the existing health surface

### 5.1 Two topics, following the time domain

[ADR 1036](../adr/1036-environment-health-surface.md) §2 separates a raw per-host feed from a rolled per-site envelope; the time domain is the worked instance (`ops/time/attest/{host_id}` raw → `ops/time/health/<site>` rolled). Consistency follows it:

| Topic | Payload | Publisher |
|---|---|---|
| `ops/consistency/report/<host_id>` | `ConsistencyReportV1` — the node's own convergence record | one per node, self-published |
| `ops/consistency/health/<site>` | `EnvHealthV1{domain:"consistency"}` carrying `ConsistencyDetailV1` | one passive rollup reader per goatnet |

Both are under `ops/**`, so they are control-class traffic (ADR 1036 §3.1) and are not starved by the load they report on, and both inherit the router's standing persistence and audit capture. "When did host B fall behind?" is answered by the existing replay path with no new storage.

```protobuf
// per-node record — ops/consistency/report/<host_id>
message ConsistencyReportV1 {
  string host_id            = 1;
  string goatnet            = 2;
  string member_group       = 3;   // which intent column this node is
  // the intent this node last converged against (from the applied-intent stamp)
  string intent_content_hash = 4;
  uint64 intent_serial       = 5;
  int64  intent_generated_at_unix_ms = 6;
  string intent_commit       = 7;   // the deploying checkout's HEAD — provenance (§3.3)
  int64  applied_at_unix_ms  = 8;
  bool   intent_verified     = 9;   // signature checked at apply time; false => unreadable, never match
  repeated ComponentStateV1 components = 10;  // ledger tips
  repeated RenderStateV1    renders    = 11;  // config render hashes (§5.2a)
  int64  collected_at_unix_ms = 12; // when these facts were read off the node
  string collector_note      = 13;  // why a collection is partial, when it is
}
message ComponentStateV1 {
  string component     = 1;
  string artifact_type = 2;   // image|config|acl|data|cert|forward-only (ADR 1060)
  string ref           = 3;   // deploy-history.jsonl tip
  string prior_ref     = 4;
  int64  ts_unix_ms    = 5;
  bool   reversible    = 6;
  string reconciled    = 7;   // confirmed | diverged | unreconciled
  string live_ref      = 8;   // what the running container reports
}
message RenderStateV1 {       // one watched render, current-vs-converge (§5.2a)
  string component           = 1;
  string path                = 2;
  string sha256              = 3;  // the file NOW; "" when unhashable (note says why)
  string recorded_sha256     = 4;  // what the last converge rendered
  int64  recorded_at_unix_ms = 5;
  string note                = 6;  // "missing" | "unreadable" | a converge-time reason
}
```

This is a sketch. The wire types are the `goat-consistency-wire` crate (`ops/consistency/rust/goat-consistency-wire/src/lib.rs`); read it, not this sketch, for field numbers. The consistency arm on `EnvHealthV1` is field 15.

- **`reconciled` / `live_ref`** give the publisher's reconciliation (§5.2) somewhere to land. Anything other than `confirmed` grades that component `unreadable` rather than as a converged fact.
- **`collected_at_unix_ms` / `collector_note`** ground the record itself: `applied_at` says when the node last converged, not when these facts were read, and a partial collection (an unreadable ledger, no docker socket) must say so rather than publish a shorter list that looks complete.
- **`ConsistencyDetailV1`** carries `nodes` plus what ADR 1109 P15 requires of a fleet reading: `goatnet`, the generation it graded against (`current_serial` / `current_content_hash`), `nodes_expected` / `nodes_reporting`, the intra-goatnet skew band (`skew_band_generations` / `skew_band_seconds`), and `intent_readable` / `intent_note` for the case where the rollup could not read or verify intent and therefore refuses to grade.

The skew band is the spread across a goatnet's own nodes, which no per-node grade can express. A zero-wide band means every reporting node holds the same generation — **whether or not it is the current one.** "They agree" and "they are right" are different findings.

**The record carries facts, never verdicts.** No grade appears in `ConsistencyReportV1`. A node knows what it is running and which generation it converged against; it does not know whether that generation is current, because that is a comparison against a tree it may not have. Grading happens at the rollup and at every reader, at read time, never stored.

### 5.2 Where the node's facts come from

- **Component refs.** `/var/lib/goat/<component>/deploy-history.jsonl`, written by the `rollback-ledger` meta-dependency of every deploying role. The publisher reads tips; it writes nothing.
- **The applied-intent stamp.** `/var/lib/goat/intent/applied.json`, written by the same `rollback-ledger` shared role: goatnet, member group, intent serial + content hash + `generated_at`, the deploying checkout's commit, `applied_at`, and whether the signature verified. Putting it in that role means it inherits universal coverage from a gate that already exists (`check-rollback-coverage.sh`).

**Ordering.** Meta-dependencies run at role *entry*, so the stamp (like the ledger row) records what the run *intends* to converge to, not a post-success fact. The publisher therefore reconciles each component against the live container before reporting, so an optimistic row from a failed mid-deploy shows as `unreadable` for that component rather than as a converged fact. A collector that cannot read the container list reports "could not look", never "nothing is running".

**The publisher and the rollup** are siblings of `goat-time-health` / `goat-mesh-health` (ADR 1036 §9): Rust, mTLS, publishing nothing but their own reading and persisting nothing. The rollup reads its goatnet's `intent.json` from disk, verifies the signature at read, and refuses to grade against an unverified intent.

### 5.2a Config render hashes — the watch-set (default-off)

Refs alone cannot show a hand-edited *rendered file* — a compose file, a templated config. The watch-set makes config-class drift observable.

**The declaration is data, in the pin table.** A component's watch-set is `render_watch:` in `ops/pins/fleet-pins.yaml` — the third declared join key the table owns, beside `var` (↔ `pins.yml`) and `ledger_component` (↔ `/var/lib/goat/`). A watch-set in role defaults would be invisible to IntentV1; one in `group_vars` would be a per-member copy of a per-component fact. Paths may embed `{{ vars }}` (`/opt/{{ zenoh_stack }}/docker-compose.yml`) and stay literal in the artifact; only the deploying host can resolve them. The generator refuses a `render_watch` on a `ledger_component: null` row.

**Expected render identity.** IntentV1 carries *which* files are a component's render identity, and deliberately not their hashes: a per-host render depends on host vars and env lookups no repo-side reader may evaluate. The baseline is node-scoped — `/var/lib/goat/<component>/render-manifest.json`, written by the `rollback-ledger` role's `render-manifest` task file, which an adopting role includes **at the end of its tasks, after every template has landed**. Position makes it a post-render fact. The write changes bytes only when the baseline moves, and a declared file the converge cannot hash is recorded with an empty hash and a note, never omitted. `goat-zenoh` is the first adopting role; it watches the compose file both its modes render. The assembled `config.json5` is deliberately not watched: per-org mode legitimately rewrites it at runtime, and watching a legitimately-mutating file would grade every legitimate change `unaccountable`.

**The comparison.** The collector hashes each manifested file now and publishes both hashes (`RenderStateV1`); `account_for_render`, in both executions and corpus-pinned (`testdata/cases/renders.yaml`), classifies the pair: agreement is `resolved`; disagreement is `unaccountable`; a file the converge rendered that is now gone is `unaccountable` too; a failed read or missing baseline is `unreadable`. A declared set with nothing recorded grades `indeterminate` — the recorder is default-off (`consistency_render_manifest_enabled: false`), so every node is in that state until a recording converge runs. The rollup carries `renders_declared` / `renders_observed` per node.

**Remediation is `reconverge`, not `rollback`.** `rollback.sh` targets the component's *prior* ledger generation, but a hand-edited file belongs to the *current* one. The evidence — the hash pair — is already on the wire and in retention before the grade exists, so re-running does not destroy it. The next reconciliation overwrites anything not in git, and here that overwrite is the fix.

**Limits.** (1) The reader joins render records at component granularity; it cannot match an unresolved declared literal to a recorded resolved path, so it compares declared and recorded counts. (2) A hand-edit made and reverted between collector passes is invisible. (3) A mutation that also rewrites the render manifest is invisible: this is drift observability against operator error, not tamper evidence against an adversary with root — the same entitlement as the ledger's.

### 5.3 Honesty mapping — grade to the 5-state colour

The mapping lives in `grammar.yaml` and exists in exactly one place:

| Grade | Colour | Why |
|---|---|---|
| `match` | **green** | grounded agreement |
| `declared` | **green**, with a `declared` badge | a declared deviation is a correct state; colouring it amber trains people to ignore amber (ADR 1036 §3.4) |
| `SKEW` | **amber** | a grounded, real difference |
| `unaccountable` | **red** | a node runs something no generation explains, or its intent signature did not verify |
| `unreadable` | **idle** | read, not evaluable — ungrounded, never green |
| `indeterminate` | **idle** per node; **unmetered** for a scope where nothing reported (`n=0`) | absence of signal is `unmetered`, not green (ADR 1036 §6) |

- **A node that has not reported is `indeterminate`/idle, never compliant.** It also counts against wave promotion, never toward it.
- **Age is not staleness when nothing changed.** A node holding the current generation is `match` however old that generation is. Intent age is reported always and graded only when the node's generation is not the current one. Grading age unconditionally would produce a red fixed by committing a no-op change.

### 5.4 What it replaced

The stage-1 committed report (`ops/pins/parity-report.md`) is deleted; its `--check` gate is the `generate-pins.py --check` gate on `intent.json`, which additionally schema-validates the document and refuses a body whose `content_hash` does not match it. `ops/pins/report.py` remains as a stdout-only matrix for the terminal, with no committed output. `ops/scripts/parity-report.sh` is the operator path to the live join, with `--stage1` for the repo-side half; when the fabric is unreachable it says so and names the command rather than falling back to the repo render, because "the repo says X" and "the fleet is X" are different claims.

A change that lands a successor surface retires or visibly downgrades its predecessor in the same change.

---

## 6. Idempotence alignment — every verdict points at a re-convergence

### 6.1 The rule

**Every non-matching graded row carries a remediation pointer, and the pointer is always one of: re-run the named idempotent play, regenerate, roll back, or declare. Never a bespoke mutation.**

Operators must be able to re-run any playbook without fear, and drift is fixed by re-running, not by hand: if it is not in git, the next reconciliation overwrites it. A consistency surface that suggested a targeted fix would be manufacturing the next `unaccountable` row, with the authority of a dashboard.

### 6.2 The closed set

| Kind | Pointer | Fires when |
|---|---|---|
| `reconverge` | `ops/scripts/ansible-preflight.sh -i inventory/mgmt-hosts-mesh.ini <env>-refresh.yml -e target=<member>` | a node is behind the current generation — the ordinary `SKEW` |
| `regenerate` | `python3 ops/pins/generate-pins.py` | repo-side drift: the artifact disagrees with its inputs |
| `declare` | add a `deviations:` stanza with a `reason:` to `ops/pins/fleet-pins.yaml` or `ops/pins/parity-capabilities.yaml` | the difference is deliberate and undeclared |
| `rollback` | `ops/scripts/rollback.sh <component>` | `unaccountable`: a node is running an unexplained ref |
| `restore` / `rotate` | the component's `restore_pointer`, or `goat admin rotate <anchor>` | `unaccountable` on a `data` or `cert` artifact, where rollback is not real |

The last row is [ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md)'s artifact-type dispatch, surfaced. The library reads `artifact_type` off the ledger record and emits the pointer that type dispatches to: `image` → automated rollback, `config`/`acl` → guided with the revert SHA, `data` → **refuse** plus the restore pointer, `cert` → **refuse** plus the rotate pointer, `forward-only` → refuse. [ADR 0859](../adr/0859-easy-rollback-engineering-principle.md) is explicit that conflating these into "rollback" creates an unsafe impression. The `refuses: true` flag rides in the pointer, so a refusal is a rendered fact rather than a command that fails when pasted.

### 6.3 The pointer is data

`remediation` is a field the library emits on `GradedRow`, not prose a renderer composes:

- Every surface shows the same command for the same condition; none of them writes the sentence.
- The library **refuses to emit a row that needs a pointer and has none** — a schema error. A new grade without a remediation kind cannot ship.
- The corpus pins pointers alongside grades, so "what does the surface tell an operator to do" is reviewed as a diff.

### 6.4 What is banned

No surface in this design executes anything. The console renders a copy-pasteable command; it does not run it, offer to run it, or hold a credential that could. Observability yes; authority no.

---

## 7. The consistency console

One binary, `ops/consistency/rust/goat-consistency-console/` (axum, server-rendered HTML); image `ops/consistency/Dockerfile.console` built by `.github/workflows/consistency-console-image.yml`; role `ops/ansible/roles/consistency-console/` (default-off; a `PENDING` pin is refused by the role's assert). The wire types are the `goat-consistency-wire` crate, linked by both the meter that writes the envelope and the console that reads it. The console's own `intent.json` read is `goat_consistency::verify::load_verified`, shared with the meter, on every render; an unverified artifact renders the refusal verbatim and suppresses the repo-vantage grid.

### 7.1 What it is

One page per goatnet, mesh-internal, read-only, that answers: *is every node running what it should, where is it not, and what do I run to fix it?* It is a **projection** — it renders `intent.json` × the `ConsistencyV1` rollup through the grading library and computes nothing else.

### 7.2 Serving

The isitup registry pattern, like every mesh-internal surface: one `isitup_services` entry in the goatnet's `group_vars`, one role rendering the container and its Traefik router (websecure entrypoint, mesh-only middleware), the FQDN in the site's certificate SAN list, and the regenerated CoreDNS zone (CI drift-gated: a declared name that does not resolve deploys dark).

- **Name:** `consistency.<site_id>.<site_dns_zone>`.
- **Access:** Traefik file-provider routing with an oauth2-proxy sidecar as the upstream, groups `PLATFORM-ADMIN,IAM-ADMIN`, with a row in `ops/permissions/group-permissions.yml` under the [ADR 1073](../adr/1073-rbac-adherence-principle.md) ratchet. The mesh-only middleware is the security boundary; the group fence is a posture choice, because the page names hosts and prints operator commands. There is no CLI-bypass router: the console validates no bearer token, so a bypass would be an ungated door.
- **Shape:** one Rust binary, server-rendered HTML with the stylesheet compiled in (no build-time asset fetch), Chainguard runtime per [ADR 0700](../adr/0700-chainguard-first.md), digest-pinned through `fleet-pins.yaml`. Rust because it holds a long-running Zenoh mTLS session and shares the grading library's Rust implementation with the meter.
- **Rollback:** the `rollback-ledger` meta-dependency, like every deploying role.

### 7.3 Zero private state

The console keeps a subscription to `ops/consistency/health/**` (and optionally `ops/consistency/report/**` for the per-node drill), the current rollup in memory, and `intent.json` + `.sig` read from disk and verified at read. No database, no history, no collector of its own; nothing survives a restart that is not re-derivable in seconds from the bus and the disk. Historical questions are answered by the existing replay path over router-persisted `ops/**`.

### 7.4 The rapid-assessment grid

Three altitudes on one page:

1. **Goatnet band** — one row per goatnet the console can see (its own; others only in aggregator mode, §7.8): the worst grounded grade, `n` reporting nodes over nodes expected, the current generation's serial, and the oldest held generation's age.
2. **Subject grid** — rows are components and capabilities; columns are the goatnet's member nodes. Each cell shows the value and its grade; each row shows the row grade and its remediation pointer. The default cell shows the human version identity — the tag, or the shortened digest when the ref carries no tag — with the full digest and verbatim ref behind an expansion. Display only: the API and the wire carry exact bytes.
3. **Row drill** — the per-node record behind a cell: ledger tip and timestamp, prior ref, the intent generation the node holds and its age, the commit that delivered it, whether the signature verified, and when the node last reported.

No percentages and no compliance scores: a percentage without a denominator is not a reading. The grid shows counts with denominators or nothing.

### 7.5 Honesty grammar, visually first-class

`unreadable` and `indeterminate` render as answers: an explicit state chip with its reason, its `n` and its age. **Nothing ungrounded ever renders green, and no cell is ever blank** — a blank cell is read as "fine". A node that has not reported occupies its column and says so. State must be legible in both themes, not distinguished by a faint tint.

### 7.6 Remediation, verbatim and copy-pasteable

Every non-matching row shows its pointer exactly as the library emitted it, in a monospace field with a copy affordance, with its `kind` and — where the artifact type refuses — the refusal and the safe alternative. The console never composes, shortens or parameterises the command and never executes it (§6.4). If the pointer is wrong, the fix is in the library and the corpus, and it reaches every surface.

### 7.7 Deep links

The console speaks the existing contract ([`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md)):

1. a backend `siblings` config block of `Option<String>` base URLs, wired through role defaults (`CONSISTENCY_SIBLING_{PANOSCOPE,STETHOSCOPE,FLASHLIGHT,PORTAL}_URL`, and `consistency_console_sibling_crossenv_url` for the cross-goatnet page);
2. a meta endpoint exposing the resolved sibling URLs (`/api/meta`);
3. an outbound `siblingUrl()` helper that returns null when a base URL is unset, so dead links are hidden rather than broken;
4. inbound `URLSearchParams` parsing on load that pre-selects the target.

Params consumed: `site=` and `domain=consistency` (so a panoscope env-band chip lands here); `key=` is not consumed, because consistency rows are components and hosts, not topics. Params emitted: `domain=consistency&site=<s>` toward stethoscope, and `peer=`/`site=` toward panoscope. Unknown params are ignored. No new param is minted.

### 7.8 The cross-goatnet aggregator

A second mode of the same binary: read-only, joining several goatnets' rollups with their committed intent to answer "is goatnet A ahead of goatnet B, and where". It may run rear-hosted or on an operator laptop, and it is never load-bearing: a cross-goatnet view may render; it may not decide (ADR 1109 §6.2; the derived read-only projection stance of [ADR 1065](../adr/1065-graph-engine-property-graph-read-model.md)).

**Three constraints.** It is the same code path with a wider input set, so it cannot develop its own semantics. It renders a goatnet it cannot currently reach as `indeterminate` with the age of its last reading, never as green or absent. It has no retirement criteria: it may be rebuilt or discarded freely and no goatnet notices.

**Mode and configuration.** `--aggregate <config.yaml>` on `goat-consistency-console`. The config (`ops/consistency/aggregator/aggregator.example.yaml`, pinned by a test) declares per goatnet a Zenoh endpoint, a cert/CA set, the committed `intent.json` path and SPKI directory, the expected `ops/consistency/health/<site>` key, and that goatnet's own `console_url`. Single mode is the same struct with one slot, and every per-goatnet grid renders through the same `goatnet_sections` path.

**One session and one state slot per goatnet.** They are separate fabrics with separate CAs. An unopenable session degrades to that goatnet's row (its bus note plus an indeterminate band cell); the page never fails whole. Verify-at-read runs per goatnet, so one tampered intent refuses its own grid only.

**Transport.** Each goatnet is reached over its own transport — a session on that goatnet's mesh, or a door that terminates mTLS at the goatnet's edge ([`env-doors.md`](env-doors.md), [ADR 1117](../adr/1117-per-env-doors-cross-env-management.md)). Never a cross-goatnet mesh bridge. The laptop form is `ops/consistency/aggregator/serve.sh`: a loopback-only bind (a laptop page has no fence, so it must have no reachable address), with one port-forward per goatnet. A hosted form is fenced by oauth2-proxy (`ops/ansible/consistency-aggregator-deploy.yml`).

**The drift-first view.** The aggregate page answers *"is there drift between the goatnets, and does it matter"* above the fold: a cross-goatnet drift panel (one summary sentence with the denominator — `N rows compared across <goatnets>: X undeclared drift · Y declared differences · Z match` — then a component-by-component table), the §7.4 goatnet band second, and each goatnet's full grid behind a drill-down that renders in full either way.

**The cross-goatnet diff is the library's own comparison.** `goat_consistency::diff_intents` composes the existing `grade_row` over a wider column set — one column per `goatnet:member`, cells copied verbatim from each goatnet's *verified* committed IntentV1 — so the verdicts are the same six grades:

- `SKEW` is a cross-goatnet difference nothing declares — the drift signal.
- `declared` is a difference the artifacts declare. **A declaration covers the columns it names, and the row is `declared` only when the columns nothing declares still agree.** A declaration about one goatnet's standby host cannot excuse a difference between two goatnets nobody declared.
- `indeterminate` is present-in-one-artifact-only, rendered as a synthetic `unset` cell. A synthetic cell is never covered by a declaration.
- **`until:` expires.** A declaration past its date stops covering its column; the row falls back to the verdict its values earn, and the generated `why` names the column and the lapsed date. The library reads no clock: `diff_intents` takes `today` from the caller, and an undated comparison expires nothing. Expiry lives in the cross-goatnet diff; extending it to the within-goatnet grid is a change to `grade_cells` in both executions and the corpus.
- `node_shape`, `retired` and `ungoverned` are per-goatnet assertions and are not compared.

The diff is computed at render time and stored nowhere. The merge exists only in the Rust library; its verdicts are `grade_row`'s, which stays corpus-pinned in both languages.

**The page states which generation it was computed for.** The console serves the intent artifacts placed on its host at deploy time, so the drift panel names each goatnet's serial beside its age (amber past a day) and the date the comparison was dated for. `aggregate-v1` carries the same as `cross_env.observed_generations[]` and `cross_env.dated_for`. Delivering fresh artifacts without a redeploy is [ADR 1125](../adr/1125-fleet-reconciliation-join-and-graduation-sets.md)'s business ([`fleet-reconciliation-join.md`](fleet-reconciliation-join.md)).

**The `aggregate-v1` JSON API.** `/api/aggregate` (both modes) and `/api/aggregate/<name>`: self-dating (`generated_unix_ms`) and shape-versioned (`schema: "aggregate-v1"`). Per goatnet: `site`, `reachability {state: live|stale|never-heard, last_heard_age_s, bus, note}`, `intent {verified, serial, content_hash, generated_at, note}`, and `convergence` (counts with denominators, per-grade counts, the worst-grounded-grade collapse, and per-node graded rows carrying `why` and `remediation` verbatim). Top-level `cross_env` is the `diff_intents` report. An unreachable goatnet is an entry with `state: never-heard` (age null) or `stale` (age stated), never omitted; a lone goatnet's `cross_env` says `comparable: false` with the reason.

**Both pages exist, and link to each other.** The aggregate view is a content superset. The per-goatnet page exists because it survives isolation (§7.9): with the WAN cut and the aggregator gone, a goatnet's own console still answers from a local file and a bus that never leaves the site. Each site page links to the cross-goatnet page, and each goatnet section of the cross-goatnet page links back; the link title carries that reason. On a status page the two rows are named for scope — **`Consistency — this goatnet`** and **`Consistency — all goatnets`** — each with a one-sentence description of what the other does not survive.

**The aggregator's own pin.** Aggregate mode templates `consistency_aggregator_image`, not `consistency_console_image`. Its host may not be a member of any goatnet in `fleet-pins.yaml`, and then nothing generates its pin or grades its drift; the image must still be a ref the `consistency-console` component declares, pinned by digest.

**Presentation rules** (the library's verdicts and the API are untouched by all of these):

- why/remediation render as full-width sub-rows under the row, never as a column;
- a match row is one compact line; when every column agrees, one spanned `all: <value>` cell;
- templating tokens (`{{ var }}`) render subdued as `<var>`, verbatim on hover;
- row order is undeclared drift first, then the remaining grades worst-first, matches last;
- grade chips use plain words — `SKEW` → `undeclared drift`, `unaccountable` → `unexplained value`, `unreadable` → `unreadable value`, `indeterminate` → `no reading` — with the grammar term and its meaning on hover; a test fails any grade that falls through to the unknown-grade fallback;
- the default `why` is the conclusion in plain words, with the library's generated sentence and the declarer's full reason behind an expansion;
- remediation renders as the kind's safety label from the closed set (`reconverge` = safe: idempotent re-run; `regenerate` = safe, repo-side; `declare` = policy edit, a reviewed PR that changes no host; `rollback` = re-deploys the prior ledger generation; the refusing kinds say why) plus one action phrase, with the exact command one click behind; an unknown kind gets no invented label;
- a differing component row carries a distance line only where the goatnets' own ledgers ground it (`X ahead since <date>` when one goatnet's confirmed tip has the other's current digest as its `prior_ref`); otherwise per-goatnet deploy dates, or `versions differ — how far is not statable` with the reason;
- tables are fixed-column grids with a sticky header, one line per cell, explanations capped at about 90 characters wide and two lines, `declared` rows collapsed to one line, page width capped at 1280px.

### 7.9 Island-mode test

With the aggregator down, the WAN severed and the rest of the fleet unreachable, a goatnet's own console still renders that goatnet completely, because its two inputs are a file on the management host and a bus that never leaves the site. Only the cross-goatnet row is lost, and it renders `indeterminate` with the age of its last reading.

The converse: **kill the console and nothing changes.** No node's convergence, deploy, rollback or health publication depends on it; no enforcement path reads it. The aggregator binary declares no publisher, puts nothing on any fabric and writes no file, and a test pins that.

### 7.10 Brokerless safety check

Read against [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md), because a consistency console is exactly the shape a re-introduced broker would take. Each node self-publishes its own record (no central collector); the rollup is a passive reader with no authority (losing it loses the view, not the facts); the comparison is computed at read time and stored nowhere; no enforcement path reads any of it. The console adds a vantage, not a chokepoint.

### 7.11 DIL declaration

- **Disconnected:** the console serves its own goatnet from local facts (§7.9). Cross-goatnet rows render `indeterminate` with their age.
- **Intermittent:** the rollup is a small periodic envelope; a missed publication ages a reading rather than dropping it, and the grade moves toward `idle` as it ages.
- **Low-bandwidth:** payloads are refs, hashes and grades. The page is server-rendered with no build-time asset fetch; the per-node drill is fetched only when opened.
- **Named deviation:** the per-node publisher emits on an interval even when the fleet is quiet, because no operational traffic implies "this node's intent is current". The interval jitters and decays under failure (ADR 1109 P9).

### 7.12 Complexity budget

Per [ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md) D7:

- **5-minute and 30-minute paths:** unchanged. The applied-intent stamp is written by a role the operator already runs and never has to know about.
- **1-day path** (second site + federation): one `isitup_services` entry and one group_var (whether the console deploys), default off.
- **1-week path** (troubleshooting): one consumer-facing surface and one new operator concept — the consistency grade. The part an operator must learn is the first-action difference between `SKEW` (re-run) and `unaccountable` (look before re-running). What pays for it: the archaeology step ("read sixteen var files, then SSH four hosts") goes away for the class of question it answers, and a verdict comes with a command. The page teaches its own vocabulary (§7.8 presentation rules).

### 7.13 The Release readiness view (`/release`)

A second page on the same binary, behind the same fence, in both modes: the [ADR 1120](../adr/1120-release-graduation-ladder.md) graduation ladder, rendered. [`release-graduation.md`](release-graduation.md) §8 is canonical for what it shows. It is on this console because "is this goatnet running what it should" and "what should it run next" are one operator's question asked at two moments.

- **Computes nothing.** Every grade, blocker, soak clock and ledger join is computed by `ops/release/graduate.py readiness` — the same `build_status()` and `gate_failures()` the `graduate` subcommand refuses on — and arrives as a `release-readiness-v1` document. The ladder's vocabulary is not the grammar's, so `release.rs` carries one translation table, and an unknown term renders as itself on a neutral chip.
- **Zero private state.** The document is read from disk on every render, never cached. It is placed beside the intent artifact by the role and is derived at deploy time, not committed: it stamps the generating checkout's HEAD, so a committed copy could never pass a regenerate-diff gate.
- **Honesty.** The document's age and source commit render at the top, and the page degrades with age — `fresh` / `stale` (7d) / `very stale` (30d), plus `clock skew` and `age unstatable`, each with the instruction to redeploy the console. An absent or unparseable document refuses and names the path, because an empty ladder reads as "nothing to release". A ledger question that could not be asked renders "not asked", distinct from "no deploy recorded". A soak whose clock cannot start renders no countdown; a waived soak renders as waived-by-whom. The "commits since the pinned build touched this component's build inputs" reading renders unknown-with-a-reason wherever the image workflow's `paths:` filter cannot be evaluated — never as zero.
- **Executes nothing, writes nothing.** `/release` and `/api/release` are reads. Kill the console and no graduation, deploy or rollback changes.

---

## 8. Non-goals

- **No remediation execution.** Nothing here runs a command, schedules one, or holds a credential that could.
- **No console-owned state.** No database, no history store, no "last known good". Once it stores something nobody else has, it is an authority with a front end.
- **No second comparison.** A surface that needs a verdict the library does not emit files a corpus case, not a local function.
- **No intent for everything.** IntentV1 covers governed pins, curated capabilities, node shape and retired keys, not all of `group_vars`.
- **No compliance scoring.** No percentages, no grades-of-grades, no fleet health number. Counts with denominators, or nothing.

---

## 9. Build plan

Each increment is useful alone and retires what it supersedes in its own change.

| # | Increment | Gate | Retired in the same change |
|---|---|---|---|
| **CF-1** | **IntentV1** — JSON Schema, the generator's second output, the committed artifact per governed goatnet, `--check` in the `fleet-pins` job | artifacts render, validate against the schema, and `--check` fails on a hand edit (both as a regenerate-diff and as a `content_hash`-vs-body mismatch) | nothing (additive) |
| **CF-2** | **The grading library** — `ops/consistency/` package, `grammar.yaml`, the golden corpus, `report.py` re-pointed at it | the stage-1 output regenerates byte-identically; the corpus is green and goes red on a precedence edit | `report.py`'s private resolution and grading semantics |
| **CF-3** | **ConsistencyV1** — the proto arm (15), the applied-intent stamp in `rollback-ledger`, the per-node publisher, the rollup meter, `goat health --domain consistency` | the Rust execution passes the shared corpus, including reproducing every committed generation's `content_hash` byte-identically; a two-peer Zenoh test publishes a real `ConsistencyReportV1`, receives it and grades it | the committed stage-1 report (§5.4) |
| **CF-4** | **Config render hashes** — the declared watch-set (§5.2a) | a hand-edited watched file grades `unaccountable` with the `reconverge` pointer through recorder → collector → shared library, pinned as tests and corpus cases in both executions | the "config drift is invisible" caveat |
| **CF-5** | **Sign the intent artifact** — `intent.json.sig`, the CI signature gate, verify-at-read in every consumer | a tampered body, a stripped signature and an unknown CA each refuse in every consumer in both languages; pinned as tests (`test_signing.py`, the meter's `tests.rs`, `verify.rs`) | the interim "the CI gate is the integrity story" posture |
| **CF-6** | **The consistency console** (§7) | the page renders a goatnet's grid from a real `EnvHealthV1{domain:"consistency"}` envelope published between two Zenoh sessions and received through the console's own ingest path; the §7.9 island render is a pure function of a verified file plus in-memory bus state | — |
| **CF-7** | **The cross-goatnet aggregator** (§7.8) | two goatnets render from both committed signed artifacts over two real Zenoh transports; never-heard and stale render indeterminate-with-age; one goatnet's tampered intent refuses its own grid only; killing the aggregator is a no-op by construction (pinned) | — |

FM-2's site-local intent replica consumes CF-1's artifact and CF-5's signature and is planned in [`fleet-management.md`](fleet-management.md).

---

## 10. Composition

[ADR 1110](../adr/1110-typed-consistency-foundation-and-console.md) · environment-parity §5 (the model) · [`fleet-management.md`](fleet-management.md) / [ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md) (P5 intent-as-data, P7 intent age, P13 four-axis declaration, P14 fail-closed verification, P15 honesty) · [ADR 1036](../adr/1036-environment-health-surface.md) (the envelope and the plug-in path) · [ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md) (the ledger and its dispatch) · [ADR 1082](../adr/1082-name-fallback-resolver-signed-snapshots.md) (the signing and distribution pattern) · [ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md) D7 · [`zenoh-surface-deep-links.md`](zenoh-surface-deep-links.md)

### The four-axis declaration (ADR 1109 P13)

| Axis | Value | Why |
|---|---|---|
| Transport | `pull` | readers subscribe; nothing is pushed at a node, and the node's own publication is a self-report |
| Partition-mode | `activate-on-arrival` | a reading is a reading whenever it arrives |
| Artifact class | `config` for the intent artifact; the observed classes are whatever the ledger records | intent is a rendered config object; rolling it back is a git revert plus a re-run |
| Blast unit | `node` | each node publishes only about itself |

## 11. Status and gates

**Declared-but-never-deployed is outside the on-host comparison.** On a host, both sides of the comparison move only when somebody deploys:

| | file | written by |
|---|---|---|
| declared | `/etc/consistency-console/intent.json` | `copy:` from the deploying checkout |
| applied | `/var/lib/goat/intent/applied.json` | the same deploy, through `rollback-ledger` |

A merge nobody deploys moves neither, so they agree and the surface grades green. ADR 1109 FM-2 (a node fetching intent from a site-local replica) closes this and is designed, not built. Until then the reader that sees both sides is one holding a current checkout and host access: `ops/scripts/deployed-vs-declared.py`. It needs no fabric; `fleet-pins.yaml` already carries the join key (`var` → the declared pin, `ledger_component` → the ADR 1060 ledger directory). Its rules:

- **A ledger tip is a claim, not an observation.** `deploy-history.jsonl` records that a deploy ran; nothing removes it when a component later leaves the host. So the reader falsifies every image row against the host's running containers in the same pass, by the rule `consistency-snapshot.py:reconcile()` applies on the node: match on the whole ref, on the digest, or on the bare image name. A different build running belongs in the drift comparison; nothing by that name running makes the row `LEDGER ONLY, NOT RUNNING`. If the container list cannot be read, the report says every row is the ledger's word for itself.
- **Compare on the `@sha256:` digest, never the whole ref.** The pin holds `{{ ghcr_org }}` unresolved and the host's copy has it resolved, so string equality would report drift on every row.
- **Unreachable, missing ledger, or a pin with no digest grades `indeterminate`, never ok.**
- Non-image artifacts (packages, kernel modules) keep ledger grading; there is no container to look for.

Where this reader runs on a schedule is open: CI holds the repo but cannot reach mesh hosts, and a management-host timer reaches the host but has no current checkout. FM-2 is the real fix.

**Gate to canon:** CF-1 through CF-3 deployed on at least one goatnet with one measurement artifact showing a true consistency row that was previously invisible, a named failure mode per increment, and a stranger able to redeploy from this doc alone.

**The claim this design does not make:** any statement of the form "goatnet X is N% consistent". The entitled claims are graded rows with their denominators.

**Open items:**

1. **Publish interval and staleness threshold** for `ops/consistency/report/<host_id>` — the age past which a node's record downgrades to `idle`. ADR 1036 leaves the same question open for `ops/time/attest`; it should be answered once for both.
2. **Bringing the aggregator's host into `fleet-pins.yaml`** as a member that declares `absent:` for the components it does not run, which replaces the §7.8 pin check with full governance.
3. **Per-component bump provenance in IntentV1**, so a generation- or commit-count distance between goatnets becomes computable; today it is not, and the console does not fake it.
