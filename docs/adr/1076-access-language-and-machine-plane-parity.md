# ADR 1076 — The access language (query/mutate/simulate/visualize) + machine-plane parity as a commitment

| Field | Value |
|---|---|
| Status | Accepted-direction (identity-owner decision 2026-07-09); declared + build-sequenced |
| Date | 2026-07-09 |
| Repo scope | goat core (portal API, goat-cli `access`, the COP web surface) |
| Related | [ADR 1074](1074-data-access-role-architecture.md) (the grammar this language operates on), [ADR 1075](1075-producer-data-labels-and-access-request-loop.md) (grant rows + labels), [ADR 1026](1026-data-access-common-operating-picture.md) (the COP — the visualization home and the existing `goat access who/topic/why`), [ADR 1027](1027-partner-release-set-governance.md), F-207 (mutation audit) |

## TL;DR (for a layperson)

Two decisions. First: managing access should feel like Snowflake's — one small command language that can ask ("who can see this?", "what changed since June?"), change ("grant Acme this stream until March"), rehearse ("if I granted this, what would change?"), and draw the picture — usable identically by a person at a terminal, a program calling the API, and the web view. Second: the machine gap closes. Machines will obey the same grants humans do; the enforcement travels as encryption keys instead of web-page checks, because our data plane deliberately has no central engine to intercept each request — but the policy being enforced is one and the same list of grants.

## Part 1 — Machine-plane parity is a commitment, not a contingency

**Principle: one policy, one language, both planes; only the enforcement transport differs.**

- The grant rows and labels (ADR 1074/1075) are the single policy for humans AND machines. No machine-only policy dialect ever.
- Why the transport must differ (and a fork stays rejected): human reads pass through goat-owned surfaces (per-request check possible); machine reads pass through the stock router whose ACL is static, restart-required, exact-CN. The ADR 1025 write trick — pre-cut one rule per identity — does not transfer to reads, because read grants are **relational** (consumer×producer pairs cannot be pre-cut). Therefore machine-read enforcement = **possession of keys distributed per the same grant rows**.
- **M2 (grant-keyed payload encryption) is upgraded from requirement-gated to a committed roadmap item**, sequenced after the resolver + labels/grant-loop build-out (it consumes their rows). The old tripwire — before one goatnet hosts two mutually-non-releasable orgs — remains as a **hard floor** on sequencing, not the trigger.
- Interim rungs stay as declared (SoT `wire_read_posture.plan`): M1 static carves for slow-changing sensitive streams; **M1.5 (new)**: the goat client libraries/SDK refuse to subscribe beyond the caller's grants — compliance-floor only, honest about its trust model (it binds well-behaved code, not adversaries; adversary-proof is M2's job).
- **Current state (2026-11-08): machine parity is built.** M2 is built ([ADR 1094](1094-grant-keyed-payload-encryption.md)): `grant-keyd` seals each release set to the audience it reads from the same grant rows the human surfaces check. M1.5 was the stopgap while M2 landed and is not needed.

## Part 2 — The access language

One functional verb set, exposed identically as `goat access <verb>` (CLI) and portal API endpoints (JSON-first; the CLI is a thin client, per the existing goat-cli pattern). Builds on the already-built `goat access who|topic|why` (ADR 1026 G.4) rather than replacing it.

| Verb | Does | Snowflake analog |
|---|---|---|
| `show grants [--of <principal>\|--on producer:<K>\|--org <O>]` | enumerate grant rows (never simulate) | `SHOW GRANTS` |
| `can <principal> read\|write <key> [--as-of <date>]` | verdict + the four-layer why-chain naming the deciding record | none — Snowflake can't explain across systems |
| `why <principal> <key>` | the existing decision trace (kept) | none |
| `simulate --grant/--revoke/--label ... --then can ...` | dry-run a mutation against a copy of the rows; show the delta | limited (`SYSTEM$...` nothing equivalent) |
| `grant view --on producer:<K> --to org:<O>\|principal:<id> --until <date> --because "<why>"` | mint a grant row (producer-approved path or operator, F-207-audited) | `GRANT ... TO ROLE` |
| `revoke ...` | end-date a row (reversible, never delete — G6) | `REVOKE` |
| `label set producer:<K> access=controlled --because "<why>"` | apply/void a label lane value (justification enforced on restriction) | `ALTER ... SET TAG` |
| `diff --since <date> [--of\|--on ...]` | what changed: grants, labels, implications | ACCESS_HISTORY, weaker |
| `graph [--focus org:<O>] --json\|--dot` | export the access graph for the COP web surface / external tooling | Snowsight, closed |

**Where it beats the model it mirrors (each claim grounded in something we already have):**
1. **Why-chains** — every answer names the deciding record at each of the four layers (mesh/identity/wire/release), built in the COP today.
2. **Time as a first-class axis** — rows are effective-dated (ADR 1051), so `--as-of` answers "who could see this on June 1" and `diff --since` is native; grant history is queryable, not log-archaeology.
3. **Two-plane** — one language answers for a browser human AND a cert-holding process; Snowflake has one engine and one plane.
4. **Simulation** — rehearse a grant before making it; the enumerable-rows design makes this a pure function over data.
5. **Policy in git** — the SoT and label defaults are diffable, reviewable, CI-guarded files; the engine-resident half (grant rows) is exportable via `graph`/`show`.
6. **Open output** — `--json` on every verb; the visualization (the ADR 1026 COP web surface) is a client of the same API, not a privileged UI.

**Visualization** is NOT a new surface: the COP web view (ADR 1026 "See it", R.3) renders `graph` + `show` + `why`, with the existing deep-link contract into panoscope/id-console. This ADR adds the mutation verbs and the language discipline to that already-decided home.

## Alternatives considered

- **A SQL dialect.** Rejected: nothing here is a table scan; verbs over rows are smaller, and the CLI/API duality matters more than query composability we don't need yet.
- **A policy rule language for the machine plane.** Rejected again (XACML lesson; SP 800-178): rules are un-auditable by enumeration and there is no engine to run them in-path.
- **Forking the router to add read checks.** Standing rejection (ADR 0858, brokerless-consequences).
- **Leaving machine reads as a contingency.** Rejected (identity-owner decision, 2026-07-09) — parity is the goal; this ADR commits the path.

## Consequences

- **+** One mental model and one command set for every access question and change, human or machine, present or historical or hypothetical.
- **+** The machine gap has a committed closure path that reuses — not duplicates — the policy layer.
- **−** Build cost is real and sequenced in the follow-ons (§0d): mutation verbs need the portal grant/label endpoints (ADR 1075 items) first; M2 needs key distribution + consumer library work; `--as-of` needs the effective-dated store fully populated.
- **−** M1.5's compliance-floor is not security against adversaries and every doc that mentions it must say so.
