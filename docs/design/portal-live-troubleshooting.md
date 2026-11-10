# Portal live-troubleshooting surfaces

**Status:** Surfaces 1+2 implemented; Surface 3 design-of-record + stub.
**Scope:** goat-zenoh-portal (Block 82A trial fabric).
**Composes with:** [`portal-config-workbench`](./portal-schema-publishing.md) (static half), [`pubsub-best-practices.md`](./pubsub-best-practices.md), [`zenoh-trial-fabric.md`](./zenoh-trial-fabric.md).

## Why this exists

The portal's [config workbench](https://portal.nova.netbird.nova.example/peering/workbench.html) is the **static half** of vendor self-service: lint the `zenohd.json5` before the partner stands the container up. The workbench cannot reach the situations a vendor actually hits after they hit Enter:

- "I started the publisher and it crashed with this wall of Rust." (log analysis)
- "I think it's connecting but I see nothing — is the router even reachable from my machine?" (connectivity)
- "My publisher is publishing but my subscriber sees nothing — is the wire actually carrying anything?" (round-trip)

Each surface in this doc answers one of those three questions. The shared trait: **paste-driven, no inbound listener exposure**, no auth (pasted content carries no secrets, same posture as `/api/fabric/zenohd-config`).

Surfaced 2026-05-23 by a real Windows-WSL vendor publisher whose terminal output showed the classic peer-mode-listener-missing-cert error class. A 30-second log-analyzer paste would have answered "you're in peer mode, switch to client mode" instead of forcing the vendor to parse Rust crate stack traces.

## Surface 1 — log-paste analyzer

**Live state:** ✅ implemented in this track. Endpoint `POST /api/peering/diagnose-log`; page `/peering/log-analyzer.html`.

### What it does

Vendor pastes their `zenohd` or publisher stderr. Backend runs the paste through a curated catalog of error-signature regex patterns. Each match returns a structured finding with severity / a one-line description of what went wrong / the specific config line or step to change / a doc-anchor back to the canonical guide.

Same UX as the workbench: paste left, findings right, debounced re-run as the user types.

### Pattern catalog (v1)

Each pattern is a `(name, regex, severity, message, doc_anchor, fix_hint)` tuple at the top of `internal/api/log_analyzer.go`. Adding a new signature = appending one entry + one test fixture.

The v1 catalog covers the failures observed in vendor walkthroughs so far:

| ID | What we match | What it means | Pointer |
|---|---|---|---|
| `peer-mode-listener-missing-cert` | `Cannot create a new TLS listener` + nearby `No such file or directory` | Running in peer/router mode but no server-side cert files configured. Vendor is a publisher → should be `mode: "client"`. | `/peering/guide.html#flavor-a` |
| `scout-multicast-no-connect` | `Listening scout messages on` + no `connect.endpoints` | Peer-mode default, multicast scout enabled, no explicit connect target. On a remote fabric this never finds the router. | `/peering/our-config.html` |
| `tls-handshake-cert-unknown-ca` | `UnknownIssuer` / `unknown certificate authority` / `certificate signed by unknown authority` | Trust anchor not pinned. Vendor needs to download `/peering/root-ca.pem` and reference it in `root_ca_certificate_file`. | `/peering/our-config.html` |
| `tls-handshake-hostname-mismatch` | `NotValidForName` / `hostname mismatch` / `certificate is valid for ... not` | `verify_name_on_connect` is checking the DNS name (correct) but the server cert SAN doesn't match. Usually means dialing the raw mesh IP instead of the DNS hostname. | `/peering/our-config.html` |
| `tls-handshake-client-cert-missing` | `peer did not return a certificate` / `bad certificate` | Vendor's mTLS leaf cert not loaded — `connect_certificate_file` / `connect_private_key_file` path doesn't exist or has wrong permissions. | `/peering/our-config.html` |
| `tls-handshake-uri-san-rejected` | `URI SAN` + (`not authorized` / `prefix mismatch` / `denied`) | Vendor cert has URI-SAN that doesn't match their declared prefix. Mismatched manifest. | `/peering/guide.html#identity` |
| `endpoint-dns-resolution-failed` | `failed to resolve` / `name resolution` + the router hostname | Vendor's DNS can't resolve `zenoh.nova.netbird.nova.example`. They're not on the mesh, or their resolver is wrong. | `/peering/our-config.html` |
| `endpoint-tcp-refused` | `connection refused` + `:7447` | TCP reach to the router port is blocked (firewall, no route, not on the mesh yet). | `/peering/our-config.html` |
| `endpoint-tcp-timeout` | `connection timed out` + the router endpoint | Same family as `refused` but silent-drop instead of RST. Usually a firewall. | `/peering/our-config.html` |
| `python-eclipse-zenoh-import` | `ImportError` / `ModuleNotFoundError: ` + `zenoh` | Vendor running the Python first-publisher without `pip install eclipse-zenoh protobuf`. | `/peering/guide.html#flavor-a` |
| `python-version-skew` | `version` + `incompatible` + (eclipse-zenoh version mention) | Vendor running a Zenoh Python lib version that doesn't match the fabric pin (1.9.0). | (link to version-pinning doc) |
| `subscribe-no-samples` | `subscribed` + `samples: 0` + duration | Vendor subscribe declared on a literal key with no wildcard — classic S3-vs-channel mental model trip from best-practices.md. | `/peering/best-practices.html` |
| `acl-denied-publish` | `denied` / `not authorized` / `forbidden` + `key_expr` | Vendor's publish key_expr falls outside their `<prefix>/**` ACL allowance. Usually a typo. | `/peering/guide.html#acl` |
| `cargo-nonexistent` | `cargo install zenoh-cli` | Vendor copy-pasted from an old guide; this command doesn't exist (F-DOG-7). | `/peering/guide.html#flavor-a` |

Each finding carries the literal matched line (truncated to 200 chars) so the user can see the linter found *their* line, not a vague guess.

### Dimensions of implementation to be aware of

**1. The pattern catalog rots.** Zenoh's wire format and error strings evolve across versions. The fabric is pinned to 1.9.0, but vendor toolchains drift. Each pattern signature needs to be tied to a specific Zenoh version range, and the catalog itself needs a quarterly review cadence. A pattern that silently stops matching is worse than no pattern at all — it gives the user false confidence ("the analyzer found nothing, must be fine"). Mitigation: every pattern has a regression-test fixture under `internal/api/testdata/log_analyzer/`; a pattern landing without a fixture fails CI.

**2. False positives have a higher cost than false negatives.** A wrong "this is your problem" finding sends the vendor down a rabbit hole; a missed finding just means they keep asking for help, which they were going to do anyway. Prefer high-specificity regexes (literal Zenoh strings, not generic terms). Patterns that match against generic infrastructure terms (`connection refused` alone, with no `:7447` qualifier) get rejected at code review.

**3. Paste content is operator-curated, not necessarily sanitized.** Vendors paste raw stderr that can contain hostnames, IPs, sometimes file paths. The portal does NOT persist analyzer pastes by design — they're parsed in-memory, results returned, dropped. No logs, no DB writes. If we ever want to learn from common failures, a future opt-in "submit anonymized hash" surface lands separately, not as a default.

**4. The catalog is a documentation artifact too.** Vendors should be able to read the catalog before pasting anything — "what kinds of errors does this know about" is itself a debugging aid. The page renders the catalog as a collapsible reference section beneath the paste box.

## Surface 2 — connectivity probe

**Live state:** ✅ implemented in this track. Endpoint `POST /api/peering/probe-result`; page `/peering/probe.html`.

### What it does

The portal generates an opinionated bash one-liner the vendor copies, runs locally (or in their CI / on the host that's supposed to reach the router), and which emits a single-line JSON blob. The vendor pastes that JSON back. The portal returns a per-layer pass/fail with the exact remediation step for whichever layer broke.

Five layers, checked in this order (each layer's failure short-circuits subsequent layers but the JSON still carries observed values for the ones that ran):

1. **DNS resolves** — `getent hosts zenoh.nova.netbird.nova.example` or equivalent. Failure = vendor not on the mesh (or wrong resolver).
2. **TCP reach** — `timeout 5 bash -c '</dev/tcp/zenoh.nova.netbird.nova.example/7447'`. Failure = firewall / no route.
3. **TLS handshake** — `openssl s_client -connect <host>:7447 -servername <host> < /dev/null`. Records: handshake success, advertised cipher.
4. **Server cert chain validates against published root-CA** — `openssl s_client ... -CAfile <root-ca>` AND a SHA-256 fingerprint compare against the value at `/api/fabric/config`. Failure = trust anchor not pinned, or fabric rotated its CA and the vendor is on a stale anchor.
5. **Vendor's leaf cert URI-SAN matches their advertised prefix** — `openssl x509 -in <connect_cert> -text -noout | grep URI:` against the prefix the vendor types into the page form. Failure = wrong cert handed out, or vendor pasted a different manifest's bundle.

The page renders the result as a five-row table, each row colored green / red / grey-skip, with the failing row's "fix this" expanded by default and the others collapsed.

### The one-liner

```bash
$ curl -s https://portal.nova.netbird.nova.example/peering/probe.sh | bash -s -- \
    --prefix my-vendor --cert /path/to/connect_certificate.pem
```

The script is rendered by `GET /peering/probe.sh` (text/plain, no auth) and parameterized by query string — same shape as the existing `/api/fabric/zenohd-config` template generator. It pipes everything through `jq -c .` so the output is a single-line JSON object. Two flags only: `--prefix` (for layer 5) and `--cert` (path to the connect_certificate.pem from their bundle).

### Dimensions of implementation to be aware of

**1. Paste-driven IS the security model.** The portal does NOT initiate any outbound probe. We do not run nmap. We do not curl the vendor's machine. This is explicit: the only entity that proves the vendor's machine can reach the router is the vendor's machine itself. The portal interprets what they paste back. This avoids (a) needing inbound listener exposure on the vendor side and (b) any "did the portal scan me?" trust concern. The exchange is symmetric: portal hands the vendor a script + interpreter; the vendor decides whether to run it.

**2. The script is auditable.** `curl ... | bash` is the canonical "don't trust opaque scripts" footgun. We deliberately render the script as text the vendor can read first — the page shows it inline in a code block before the run-it button is offered, and the page links to the rendered script URL so it can be fetched, read, then run. The script itself is small (~60 lines of bash), uses only `getent` / `bash /dev/tcp` / `openssl` / `jq` (all standard on Linux + macOS), and has no `set -e` magic that hides what it's doing. Windows is a follow-on (a `probe.ps1` sibling will land when the first Windows vendor hits this; the JSON shape is the same).

**3. The five layers are NOT independent.** Layer 2 can succeed only if layer 1 did. Layer 3 only if 2 did. We deliberately keep each layer separately probeable in the script so a partial result is informative: "DNS resolves but TCP refused" is a very different diagnosis from "DNS doesn't resolve" even though the second blocks the first. The portal renders the layers in dependency order so the user reads top-down and stops at the first red row.

**4. URI-SAN check is local-only.** Layer 5 verifies the vendor's OWN cert against their declared prefix — it doesn't tell the portal anything about the vendor; the assertion lives entirely on the vendor side. We don't try to verify the bundle against any portal-side record because the portal doesn't have the cert (only the vendor does). This means a vendor can lie about their cert by editing the script's output before pasting — but they only hurt themselves. The trust check at the wire is done by the router during mTLS handshake, not by this surface.

**5. No retention.** Same posture as Surface 1 — pastes are parsed and dropped. No logs that include the parsed paste.

## Surface 3 — per-vendor echo loopback

**Live state:** ⏳ design-of-record + stub UI in this track. Implementation deferred — flagged as dimensions-driven decision below.

### What it would do

Vendor publishes a payload onto `<vendor>/system/echo/request/v1`. A portal-hosted "echoer" subscribes on `*/system/echo/request/v1` (across every vendor's prefix) and republishes the same payload back on `<vendor>/system/echo/response/v1`. Vendor subscribes on `<vendor>/system/echo/response/v1` and confirms the round-trip.

This answers the third operator question — "is my publisher actually on the wire?" — that neither log analysis nor connectivity probing can answer. A clean log + a green connectivity probe + a silent subscriber is the classic S3-vs-channel mental model trip, and the echo surface is the highest-confidence signal of "you are correctly on the fabric."

### Topic shape (proposed)

Both halves of the round-trip live in the vendor's OWN namespace (`<vendor>/system/echo/{request,response}/v1`) rather than in the reserved `system/**` prefix. This is deliberate:

- Vendor's `<vendor>/**` is fully theirs to publish and subscribe — no per-vendor ACL change needed.
- Portal echoer is the only one that needs new ACL: a single fleet-wide "publish allow" for `*/system/echo/response/v1` and "subscribe allow" for `*/system/echo/request/v1`. ONE ACL stanza, not N (where N = vendor count).

The `system/echo/` middle segment within the vendor's prefix is convention, not enforcement.

### Dimensions of implementation to be aware of

**1. Where the echoer lives is the load-bearing decision.** The portal is Go and has no maintained Zenoh 1.x Go client (per the airdefenders pivot to the REST plugin). The audit-subscriber is Rust + has a live mTLS Zenoh session and is the natural home. Adding an echo subscriber to the audit-subscriber binary reuses the wire credential + the long-lived session — no new identity, no new listener, no new container — but it does mean the audit-subscriber starts WRITING to the fabric (today it's pure read). That's a deliberate posture change. Alternative: a tiny standalone `goat-zenoh-echoer` Rust service. Cleaner separation, more boxes to run. **The decision should be made in a dedicated session after the first vendor actually asks for this surface — premature standup adds a runtime piece nothing uses yet.** Block 81 complexity-budget bar applies.

**2. ACL widening is one stanza, not N.** The portal echoer identity needs publish-allow on `*/system/echo/response/v1` and subscribe-allow on `*/system/echo/request/v1`. That's two access_control rules under a new identity (matching on its URI-SAN). The router config delta is small. What's NOT small: this is the first time a non-vendor identity gets a cross-vendor capability. The precedent matters — every future portal-side service ("dead-letter sweeper", "metrics aggregator", etc.) will cite this stanza. Worth recording as an ADR when implementation lands.

**3. Panoscope sees the echo traffic.** Every echo request + response shows up in panoscope as live wire traffic. For a single vendor pinging in isolation, that's fine. For a fleet of 30 vendors each pinging once a minute on a slow link, the echo wash drowns out real signal. Mitigation candidates: rate-limit per vendor at the echoer (e.g. one round-trip per 10s) AND/OR filter the echo topic out of panoscope's default view via a topic-class tag. Decide before the echoer runs continuously.

**4. Replay-on-MinIO collision.** The audit-subscriber writes every event to MinIO `audit-integrity/` with COMPLIANCE retention. Echo traffic gets written too. On the nova-a trial fabric this means echo traffic counts against the 12 GiB audit cap. A single-stanza filter rule in the audit-subscriber to skip `**/system/echo/**` would close this — but it's a structural choice (some operators may WANT echo events in the audit trail for "did this vendor's connectivity test happen at the time they said it did" cases). Default proposed: NOT filtered (we record everything), but a filter knob exists for sites that want it off.

**5. Payload shape.** Request body is opaque bytes (the vendor decides), max 4 KiB. Response carries `{request_id, received_at, echoed_payload, echoer_id}` so the vendor can correlate round-trips. The echoer's identity is published openly in the response so a vendor sees who echoed them — useful when there are multiple echoers (e.g. one per site in a future multi-site trial).

**6. This is NOT a replacement for an integration test.** The echo loopback proves wire reachability and ACL correctness. It does NOT prove the vendor's schema descriptors are uploaded, that their subscriber is using the right wildcard, or that any of the inspector/decoded-samples plumbing works. The page UI must say this clearly — green echo round-trip is a NECESSARY but not SUFFICIENT condition for working integration.

### Why the stub is in this PR

Concretely we ship: `/peering/echo.html` rendering the planned flow + the request/response topic shape + a "Coming soon — design recorded at `docs/design/portal-live-troubleshooting.md`" notice. The page is a placeholder, but its presence in the Peering nav means the next session that implements the echoer doesn't have to design the UX from scratch. The design above is the contract.

## Cross-cutting

- **No auth on any of the three surfaces.** Same posture as `/api/fabric/zenohd-config` and the workbench. Pasted content is pre-mint or freely-shared error output; no secrets cross the surfaces; partners shouldn't need a vendor row to validate their pre-mint setup.
- **No retention.** None of the three surfaces persist pasted content. Surfaces 1 and 2 parse in-memory and drop. Surface 3 (when implemented) writes echo events to the audit trail by default but that's metadata about the round-trip, not the workbench-style paste.
- **Same nav, same dropdown.** The three pages live under the Peering dropdown alongside the workbench, guide, etc.
- **Same theme + styles.** No new CSS modules. The light/system/dark toggle works the same way.

## Backlog follow-ons

- Surface 3 implementation — pick echoer host (audit-subscriber sidecar vs new service) + write the ADR for the first cross-vendor portal identity.
- Surface 1 catalog growth — quarterly review cadence; track which patterns trigger most often via the page's optional "this was helpful" telemetry (also opt-in only).
- Surface 2 Windows companion — `probe.ps1` for vendors on Windows native (not WSL). JSON shape unchanged.
- Optional retention opt-in — a future "share anonymized failure hash with the operator" toggle for the log analyzer, paired with a doc that explains exactly what hash is computed and what gets transmitted. Default-off; not in v1.
