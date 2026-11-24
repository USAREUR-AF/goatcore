# Admin-operator guide (start here)

The orientation doc for a **backend/admin operator** — the person who onboards
participants, mints their credentials, and reads the admin audit trail. Read this
first, then use the deep runbooks it links.

**Network access is its own path, and it runs through warrant.** The NetBird
dashboard is for looking. To open or close a port between groups, give a device a
group, take one away, or quarantine and release a device, change the declaration
in git as [`warrant-runbook.md`](warrant-runbook.md) describes (§4A for
membership); a converger on the management host applies it. Start any "why can't
this reach that" with `goat warrant why-blocked`, which answers without network or
credentials and usually shows the problem is DNS, membership or a dead service
rather than the ACL. Design and roll-out state:
[`mesh-change-control.md`](../design/mesh-change-control.md)
([ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md)).

## What an admin operator can do (and how the certs actually work)

| Capability | Mechanism | What you need |
|---|---|---|
| **Onboard a participant** (NetBird key + Ory login + Zenoh cert) | `goat admin onboard <handle> --env <env>` → the orchestrator hits the **NetBird API** (setup key), **Kratos** over your SSH (Ory identity), and the **portal** (Zenoh vendor cert, minted server-side) | NetBird API token + portal operator key + SSH to the mgmt host + the repo |
| **Revoke / offboard** | `goat admin onboard <handle> --env <env> --revoke` (same surfaces, backward) | same as above |
| **Rotate a cert** (re-key, same anchor) | `goat admin rotate <anchor\|alias>` | portal operator key |
| **Read the admin audit trail** (who-did-what) | `goat audit ops` (decrypts `ops/audit/operator/**`) | your `age` identity (you must be in the operator roster) |
| **Write the `ops/**` control namespace** | your **operator Zenoh leaf** (CN `operator-trial-peer`) | your operator cert (in your bundle) |
| **Admin the management consoles** (MinIO, isitup) | Ory `ADMINS` group on your login | nothing extra — log in |
| **Look at the NetBird dashboard** (`dashboard.<site>`) | Ory `PLATFORM-ADMIN` or `IAM-ADMIN`; on a goatnet under ADR 1189 your NetBird role is `auditor`: you see everything and change nothing | nothing extra — log in |
| **Review a peer group move in the identity console** | Console `PLATFORM-ADMIN` session | Open [Move Group](../../ops/goat-id-console/README.md#move-a-peer-between-mesh-groups) on the peer row. For an enforcing scope, submit the generated declaration through the goatnet configuration repository. |
| **Break-glass NetBird edits** (`admin-dashboard.<site>`) | Ory `NETBIRD-ADMIN` (a short named roster) — for what warrant does not cover; on a governed goatnet group writes there are refused | membership emergencies: [`warrant-runbook.md`](warrant-runbook.md) §4A "Emergencies" |

**You do NOT hold the offline-CA private key.** NetBird keys come from the NetBird
API; Zenoh participant certs are minted **by the portal** (it holds the CA). You
authorize those mints with *tokens*, not by signing certs yourself. The only place
the offline CA is touched is the rare `cert-sign` mint (operator leaves, device
bundles) — a separate, gated ceremony, not part of normal onboarding.

So you carry **one** cert (your own operator Zenoh leaf) + an SSH key + an `age`
identity. That's it.

## One-time setup

1. **Install the `goat` CLI** + clone the repo.
   ```bash
   # CLI: download from the mesh surface get.<env>.… (or build from crates/goat-cli)
   git clone <repo>            # you need the repo for the ansible orchestrator
   cd <repo> && direnv allow   # loads .envrc + .envrc.local.d/*
   ```

2. **Mint your own NetBird API token** (per-person, attributable — do NOT share the
   cluster one). On a goatnet under ADR 1189 your token carries your NetBird role,
   `auditor`, so it reads everything and writes nothing — which is what the `goat`
   read verbs and `goat warrant apply --mode report` need. You have `ADMINS`, so you can:
   ```
   Open the NetBird dashboard for the env → User Profile (top-right avatar) → Profile Settings → Access Tokens → Create Access Token
   ```
   Add it to `.envrc.local.d/30-site-services.envrc`:
   ```bash
   export SITE_NB_API_TOKEN='<your token>'      # alias is <ENV>_NB_API_TOKEN
   direnv reload
   ```

3. **Get your portal operator key.** It is per-person, so your Portal admin
   actions are attributed to you rather than to a shared credential. It is not in
   your bundle and not in your welcome email — you request one and a DIFFERENT
   admin approves it. Your profile's own token (the `vp_` key `goat profile init`
   installed) is enough for both of your steps. Do not name a handle: the console
   binds the request to the person your token belongs to.

   ```bash
   goat admin operator-key request --reason "the admin task that needs it"   # reason required
   # a different admin approves, then, on this same profile:
   goat admin operator-key collect <id>                            # shown ONCE
   export SITE_PORTAL_OPERATOR_API_KEY='<the op_… value>'
   ```

   Or the same thing in the console at `/operator-key`
   (`https://admin.auth.<site_id>.<zone>/operator-key`), which is the path to
   send somebody who does not have a CLI yet, and the only path on a console
   older than the CLI self-service door (F-571; there the CLI is refused
   `operator_only`, and its hint names this page as one possible way through).
   `request <handle>` files on somebody else's behalf and needs your
   operator key, not your `vp_` token.

   **Approval is the grant.** Unlike an SSH key there is no install step
   afterwards: the key is live within seconds of approval, because the portal
   re-reads its key directory while running. If it does not work a minute after
   approval, that is a fault, not a queue.

   The value is shown **once**. If you lose it, revoke and request a new one —
   a key nobody holds still works, and nobody can tell you who is using it.

4. **Set your operator attribution + audit identity:**
   ```bash
   export GOAT_OPERATOR_HANDLE=<your-handle>             # e.g. jane-doe
   export GOAT_OPERATOR_AGE_IDENTITY=~/.config/age/site-operator.key
   ```

5. **Install your operator Zenoh cert** (from your bundle — see your `NOTES.md`):
   ```bash
   mkdir -p ~/.goat/site-operator && cd ~/.goat/site-operator
   cp /path/to/bundle/{operator-cert.pem,operator-key.pem,site-ca-root.pem} .
   chmod 400 operator-key.pem
   ```
   Point a Zenoh client at it with `root_ca_certificate=site-ca-root.pem`,
   `connect_certificate=operator-cert.pem`, `connect_private_key=operator-key.pem`,
   endpoint `tls/zenoh.<site>.<zone>:7447`. `goat` reads its cert
   from the active profile/context; `goat audit ops` uses your `age` identity.

6. **Verify SSH to the mgmt host** (used for the Kratos identity create):
   ```bash
   ssh operator@<mgmt-host-mesh-ip>
   ```

## Daily operations

- **Onboard:** author `ops/operators/<env>/<handle>.yml`, then
  `goat admin onboard <handle> --env <env>` (preview with `--dry-run`).
  Bundle lands at `ops/operators/out/<handle>-bundle.md` — hand it over OOB.
- **Revoke:** `goat admin onboard <handle> --env <env> --revoke`.
- **Rotate:** `goat admin rotate <anchor|alias>`.
- **Reissue a mesh key** (expired or used up):
  `goat admin onboard <handle> --env <env> --reissue-device-key --uses <N>`,
  or **Reissue device key** on the person's onboarding panel in the console.
  The old key is revoked, a new one is minted with N uses, and an audit record
  names you, the person and the key. `--reissue-handset-key` does the same for
  a phone. Never mint one with `curl` against the NetBird API: that leaves no
  record of who asked or who the key is for.
- **Audit:** `goat audit ops` (one-shot) — who did what, decrypted to your `age` key.

## What an onboarded person actually receives

Worth knowing before you help someone, because most "onboarding is broken"
reports are really "I could not find the thing I was sent".

**The welcome email carries only credentials** — the mesh setup key, the login
link, and for fabric roles the portal API key and a one-time bundle URL. Nothing
else. **Everything else is on a guide page** at `guide.<site>.<zone>`: CLI
install, the surfaces, publishing, troubleshooting. Mesh-only, rendered per
tier, no credentials on it, so it needs no login and can be shared freely with
anyone at the same level.

That split is forced, not stylistic: everything on the guide needs the mesh, and
joining the mesh cannot live there because until it is done the page will not
load. Useful when triaging — **if the guide opens for them, step 1 worked.**

Three things that regularly surprise operators:

- **The Portal does not take the Step-2 password.** It takes a `vp_…` portal API
  key, which is in the welcome email for anyone whose role mints one. Roles with
  no fabric identity have no portal key at all, and the email now says so rather
  than sending them looking.
- **`goat admin rotate` is not a way to give someone a key.** It re-issues the
  certificate in place and returns a NEW key, which invalidates the copy in that
  person's own bundle. Rotate when you mean to rotate. To get someone a
  credential they should already have, re-send their welcome package. If
  their mesh key expired or ran out of uses, re-sending hands them the same
  dead key: reissue it instead (above), and the console re-sends the package
  with the new one.
- **A platform-admin's SSH key is not installed automatically.** The ceremony
  skips that step silently when `manifest.operator.ssh_pubkey` is absent, and
  only the person can supply a public key. They submit one and an admin approves
  it — a submitted key grants nothing until then.
- **"Their peer is connected" does not mean their computer is on the mesh.** A
  Windows user who installs the client inside WSL enrols *that Linux
  environment*, which has its own network stack — so the peer registers, reports
  `Connected`, and looks healthy to you, while their Windows browser still
  cannot open a single mesh service. The dashboard agrees with you and the
  person is still stuck. Tell it apart by the peer's `os`/`kernel_version`: a
  WSL enrolment reports a Linux kernel with `microsoft-standard-WSL2` in the
  string, usually on a Windows-shaped hostname. The fix is to install the
  **native Windows client**; the WSL peer can then be deleted. It also spends a
  device off their setup key, so check they have one left before telling them to
  re-enrol.

**A missing welcome email is now a signal, not silence.** The console refuses to
send a package that would arrive incomplete (a mesh-enrolled person with no
setup key in hand) and records it in the audit trail as `held`. If someone
reports never receiving one, check for a held send before assuming a mail
problem.

## Ownership: recorded, not inferred

Every portal slot carries `identity_id` — the Kratos identity of the person it
belongs to. Both mint paths record it. This matters to you because **ownership
is what revocation targets**: a recorded linkage makes "revoke this person"
resolve to a fact rather than to a name match between a slot alias and a handle.

Older slots may carry no linkage. Repair with the dry-run
tool (`ops/goat-id-console/scripts/slot-identity-backfill.py`), which proposes
only bindings a source of truth already proves. **Never link a slot to a person
because you recognise the name.** A wrong owner is worse than none — it hands
one person authority over another's namespace. Machine, service, demo and org
slots correctly have no person and stay unlinked.

## Reading the consistency console

`https://consistency.<site>.<zone>` (`PLATFORM-ADMIN` fence) answers *"is every node running what it should, and
what do I run to fix it?"* — one grid: components × the goatnet's nodes, each
cell graded. The six grades, and the first action each one implies
(`../design/consistency-foundation.md` §5.3;
every chip also renders its own `why` sentence):

| Grade | Color | It means | First action |
|---|---|---|---|
| `match` | green | node runs the current generation | none |
| `declared` | green + badge | a deviation that is *declared with a reason* | none — declared is correct, not amber |
| `SKEW` | amber | a grounded, real difference (e.g. an old generation) | run the remediation pointer on the row — always a named idempotent re-converge, copy-pasteable verbatim |
| `unaccountable` | red | the node runs something **no generation explains** (hand-edit, tamper, unverified intent) | investigate before re-converging — the pointer names the reconverge/rollback, but red means the evidence matters |
| `unreadable` | idle | read, not evaluable — never green | fix the read path (cert, ledger, socket) |
| `indeterminate` | idle / unmetered | nothing reported — **never** compliant | check the publisher / the router ACL |

The load-bearing distinction is `SKEW` vs `unaccountable`: SKEW is a real,
*explained* difference you re-converge; unaccountable is bytes no converge
explains. "Nothing heard" on the page is an answer, not an error — the F-177
silent-denial hypothesis is printed with it. The console executes nothing;
every fix is the pointer, run by you. (Leak register: L-001.)

## Canonical deep docs

- **Onboarding workflow (the runbook):** `operator-onboard.md` — preconditions (NB PAT, ACL taxonomy, Kratos SSH), manifest schema, failure triage, manual provisioning.
- **Design + decisions:** `../design/operator-onboard-utilities.md`.
- **Offboarding:** `../design/credential-revocation-and-offboarding.md`.
- **Rotation / re-key:** `../design/credential-rotation.md`.
- **Operator-action audit (attribution):** [`../design/operator-action-audit.md`](../design/operator-action-audit.md).
- **Operator identity / age roster:** `dev-ca-operator-onboarding.md`.
- **General admin task register:** `admin-tasks.md`.

## Two group axes — `ADMIN` (mesh) vs `ADMINS` (services)

Group membership lives on **two independent authorization axes** with two sources
of truth. They are not the same thing, despite the near-identical names:

| Axis | Field | Source of truth | Governs | The "admin" group |
|---|---|---|---|---|
| **Mesh role / node-class** | `netbird.auto_groups` | `ops/ansible/netbird_acl_taxonomy.yml` | network reachability (mesh ACL) | **`ADMIN`** |
| **Ory service permission** | `ory_site.groups` | `ops/permissions/group-permissions.yml` | web-service authz (JWT `groups` → oauth2-proxy / MinIO / goat-id-console) | **`ADMINS`** |

So to give an operator admin on the **web consoles** (MinIO, dashboard, id-admin),
put **`ADMINS`** in `ory_site.groups`. To give them an admin **mesh** group, put
`ADMIN` in `netbird.auto_groups`. The schema enforces this split — each field only
accepts values from its own SoT, so you can't cross them. (The similar naming is a
known wart; the disambiguating rename (ADMINS → PLATFORM-ADMIN) is F-213.)
