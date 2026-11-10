### goat-cli — English (en), the canonical arbiter catalog.
### ADR 1024 / docs/design/goat-cli-i18n.md.
###
### Boundary discipline: messages here are HUMAN PROSE only. Backticked
### `goat …` commands, paths, and `{ $var }` interpolations inside a
### message are CONTRACT tokens — a translator reproduces them verbatim
### and never localizes them. The machine surface (--json fields, E_*
### codes, the error:/help:/code: line labels, completion candidates,
### exit codes) is not in this catalog by rule.
###
### Proof-of-concept scope (errors.rs::GoatError::summary, rendered to
### stderr only via DisplayHuman — never serialized into --json). The
### hint() text is producer-supplied free text and is not localized in
### this slice; full externalization follows per the design doc.

# GoatError::summary — one per variant. { $context } / { $endpoint } /
# { $topic } are contract interpolations passed through verbatim.
error-profile-missing-summary = no profile for context `{ $context }`
error-profile-denied-summary = the profile for context `{ $context }` exists but is not readable by this user (permission denied)
error-off-mesh-summary = off the ZTOL — not connected to the mesh
error-router-unreachable-summary = Zenoh router at `{ $endpoint }` is unreachable
error-cert-expired-summary = mTLS cert expired or rejected by the router
error-portal-unreachable-summary = portal API at `{ $endpoint }` is unreachable
error-portal-bad-response-summary = portal answered but this goat build could not parse the response
error-portal-not-found-summary = the portal answered HTTP 404 for `{ $path }`: not registered there
error-portal-rate-limited-summary = the portal answered HTTP 429 for `{ $path }`: too many requests, retry later
error-portal-server-error-summary = the portal answered HTTP { $status } for `{ $path }`: a fault on the portal's side
error-portal-refused-summary = the portal answered HTTP { $status } for `{ $path }`: request refused
error-token-invalid-summary = portal rejected the Bearer token
error-not-admin-summary = operator-only action — you are not authorized (`goat admin` is inert for non-admins)
error-topic-conflict-summary = topic `{ $topic }` already exists
error-bundle-invalid-summary = bundle signature or expiry verification failed
error-machine-unresolved-summary = `{ $machine }` is not exactly one enrolled machine
error-mesh-peer-unresolved-summary = enrolled machine `{ $machine }` has no single mesh peer
error-netbird-failed-summary = the NetBird management API call failed; nothing was changed
error-acl-declaration-missing-summary = no ACL declaration was supplied, so there is nothing to check

# Machine-translation notice (ADR 1024 / docs/design/goat-cli-i18n.md).
# Emitted to stderr when a non-English locale that is not native-reviewed
# is active. { $lang } is the BCP-47 code; { $channels } is the
# code-assembled feedback string (email, and the partner repo when set).
# A translator may localize this notice; the email/repo pass through.
i18n-machine-translation-warning = note: { $lang } output is machine-translated and not yet native-reviewed — the English text is authoritative. Report translation errors to { $channels }. Suppress with GOAT_I18N_NO_WARN=1.

# `goat doctor` step summaries (shown in pretty mode) + remediation hints.
# Step names (ztol-membership, …) and --json field values stay invariant
# (not here). Interpolations are contract tokens passed through verbatim:
# { $host } { $port } { $endpoint } { $err } { $ms } { $secs } { $status }
# { $n }. The backticked `goat …`/`netbird …` commands are contract too.
doctor-endpoint-malformed-summary = router endpoint malformed
doctor-step0-ok-summary = local setup is ready for network probes
doctor-step0-warn-summary = local setup has checks that could not be confirmed
doctor-step0-fail-summary = local setup needs attention
doctor-step0-profile-absent-summary = no goat profile for context `{ $context }`
doctor-step0-profile-new-hint = check `goat context ls`; if this is your first setup, run `goat ca add <bundle-ca.pem>` and compare its fingerprint with your operator's, then `goat profile init <bundle.cbor>`; rerun `goat doctor`
doctor-step0-profile-absent-hint = check `goat context ls`; select an existing context with `goat context use <name>`, or run `goat profile init <bundle.cbor>` for this context; rerun `goat doctor`
doctor-step0-profile-denied-summary = goat profile at { $path } is unreadable
doctor-step0-profile-denied-hint = fix profile ownership or run as the owning user; do not reinitialize an existing profile
doctor-step0-profile-incomplete-summary = goat profile file { $file } is missing or unreadable
doctor-step0-profile-incomplete-hint = restore the named file from the issued bundle, or use `goat profile rotate <bundle.cbor>` to replace this context; rerun `goat doctor`
doctor-step0-profile-invalid-summary = goat profile has unreadable or missing endpoints
doctor-step0-profile-empty-token-summary = goat profile has an empty portal token
doctor-step0-profile-ok-summary = goat context `{ $context }` has readable profile material
doctor-step0-client-absent-summary = no supported mesh client responded
doctor-step0-client-absent-hint = install the mesh client supplied by your goatnet, start its daemon, then rerun `goat doctor`
doctor-step0-client-stopped-summary = installed mesh client did not return usable status
doctor-step0-client-stopped-hint = check `netbird status -d` or `goat client status --detail`; start the installed daemon (`sudo netbird service start` for NetBird), then rerun `goat doctor`
doctor-step0-netbird-ok-summary = NetBird daemon answered (reported profile `{ $profile }`)
doctor-step0-goat-client-ok-summary = goat-clientd answered (state `{ $state }`)
doctor-step0-management-connected-summary = NetBird reports management connected
doctor-step0-management-disconnected-summary = NetBird reports management disconnected
doctor-step0-management-disconnected-hint = check the reported management URL and `netbird status -d`; an established tunnel may still work during a management outage
doctor-step0-management-unknown-summary = configured management endpoint could not be established locally
doctor-step0-management-unknown-hint = inspect `netbird status -d` or `goat client diagnostics`; doctor will not guess a management URL
doctor-step0-tunnel-down-summary = NetBird reports no mesh address
doctor-step0-tunnel-down-hint = run `netbird status -d`; if enrolled and the daemon responds, run `goat heal`, then rerun `goat doctor`
doctor-step0-tunnel-address-summary = NetBird reports mesh address { $address }; router reachability is checked next
doctor-step0-goat-client-no-bundle-summary = goat-clientd has no imported bundle
doctor-step0-goat-client-no-bundle-hint = run `goat client bundle import <bundle.cbor>` with the client enrollment bundle, connect the client, then rerun `goat doctor`
doctor-step0-goat-client-connected-summary = goat-clientd reports a connected tunnel; router reachability is checked next
doctor-step0-goat-client-outer-disabled-summary = goat-clientd outer tunnel is disabled in netbird-only mode
doctor-step0-goat-client-down-summary = goat-clientd reports tunnel state `{ $state }`
doctor-step0-goat-client-down-hint = run `goat client diagnostics`, then `goat client up` if the bundle is loaded; rerun `goat doctor`
doctor-step0-goat-client-inner-ok-summary = goat-clientd reports its inner mesh connected
doctor-step0-goat-client-inner-disabled-summary = goat-clientd inner mesh is disabled in wg-cp0-only mode
doctor-step0-goat-client-inner-down-summary = goat-clientd reports inner mesh state `{ $state }`
doctor-step0-goat-client-inner-down-hint = run `goat client diagnostics` to inspect the inner mesh, then rerun `goat doctor` after restoring it
doctor-step0-management-reachable-summary = management endpoint answered HTTP { $status }
doctor-step0-management-auth-summary = management HTTPS endpoint is reachable (HTTP { $status } requires authorization for this probe)
doctor-step0-management-unexpected-summary = management endpoint returned unexpected HTTP { $status }
doctor-step0-management-unexpected-hint = inspect the configured management URL and ask your goatnet operator to check this response
doctor-step0-management-unreachable-summary = configured management endpoint could not be reached: { $error }
doctor-step0-management-unreachable-hint = check DNS and network access to the URL reported by the client; an existing tunnel may still work
doctor-step0-clock-ok-summary = local clock is within { $seconds } seconds of management HTTP time
doctor-step0-clock-skew-summary = local clock differs from management HTTP time by { $seconds } seconds
doctor-step0-clock-skew-hint = synchronize the device clock, then rerun `goat doctor`; certificate validation may fail while the clock is wrong
doctor-step0-clock-unknown-summary = clock skew could not be measured from a management HTTP Date header
doctor-step0-clock-unknown-hint = check the device's time synchronization if TLS or certificate checks fail
error-clock-skew-summary = the device clock is too far from management time
error-doctor-inconclusive-summary = doctor observed a failure without a proven cause
error-portal-forbidden-summary = the portal denied this account access to the probed API
doctor-step4-forbidden-summary = portal returned HTTP 403 for this profile
doctor-step4-forbidden-hint = this credential reached the portal but lacks access to the probed API; check `goat profile show` and ask your goatnet operator to verify the grant
doctor-step4-decode-summary = portal answered with a response this goat cannot read
doctor-step4-decode-hint = { $err } — update goat to the version served by this goatnet, then rerun `goat doctor`
doctor-step1-endpoint-malformed-hint = expected `<proto>/<host>:<port>`; got `{ $endpoint }`
doctor-step1-ok-summary = TCP connect to router host { $host }:{ $port } succeeded
doctor-step1-conn-failed-summary = TCP connect to router host failed
doctor-step1-conn-failed-hint = { $err } — the router TCP path failed; check the local tunnel result above, DNS and the configured router endpoint before changing enrollment
doctor-step1-timeout-summary = TCP connect to router host timed out
doctor-step1-timeout-hint = router host { $host }:{ $port } did not answer in { $ms }ms — compare from another mesh peer; check tunnel, route and router service
doctor-step1-control-only-summary = connected to the control server only — no tunnel to any of { $total } peers
doctor-step1-control-only-hint = this network lets TCP 443 out but blocks the mesh data ports, so nothing on the mesh (sign-in pages included) is reachable from this device. Ask the network team to allow outbound TCP and UDP to { $relays }, and UDP 3478 to { $stun }; direct UDP between devices is recommended too. To confirm, join a phone hotspot and run `netbird status`: peers should then connect.
doctor-offmesh-dashboard-hint = relay fleet (external vantage): https://relay-health.sandbox.zagros.goat.example
doctor-step2-endpoint-malformed-hint = fix the bundle issuer (--goat-cli-router-endpoint must be `<proto>/<host>:<port>`)
doctor-step2-ca-missing-summary = CA roots PEM missing or unreadable
doctor-step2-ca-missing-hint = { $err } — re-run `goat profile init <bundle.cbor>`
doctor-step2-ok-summary = router { $host }:{ $port } accepting TCP; TLS is tested by the session in step 3
doctor-step2-refused-summary = router TCP refused
doctor-step2-refused-hint = { $err } — router may be down; check Zenoh deploy on the mgmt-host
doctor-step2-timeout-summary = router TCP timed out
doctor-step2-timeout-hint = router { $host }:{ $port } unreachable in { $ms }ms
doctor-step2-material-ok-summary = local certificate, matching key, and CA bundle parse; certificate is within its validity interval
doctor-step2-material-failed-summary = local TLS material { $file } could not be used
doctor-step2-material-failed-hint = { $err } — inspect the named file; restore it from the issued bundle or rotate the profile
doctor-step2-material-invalid-hint = { $file } is not a valid X.509 PEM certificate; restore the issued material or run `goat profile rotate <bundle.cbor>`
doctor-step2-key-invalid-hint = mtls.key.pem is invalid or does not match mtls.cert.pem; restore the issued material or run `goat profile rotate <bundle.cbor>`
doctor-step2-cert-time-summary = local certificate is outside its validity interval
doctor-step2-cert-time-hint = run `goat profile show`; check the clock result above, then ask your operator for a fresh bundle and run `goat profile rotate <new-bundle.cbor>` if the cert has expired
doctor-step3-config-failed-summary = zenoh config build failed
doctor-step3-config-failed-hint = invalid endpoint `{ $endpoint }`: { $err }
doctor-step3-tls-config-failed-summary = zenoh TLS config build failed
doctor-step3-tls-config-failed-hint = { $err } — likely zenoh version-pin drift; verify deps match
doctor-step3-ok-summary = mTLS Zenoh session opened successfully
doctor-step3-open-failed-summary = zenoh session open failed
doctor-step3-open-failed-hint = { $err } — run `goat profile show` to check local certificate expiry; if valid, give this result to your goatnet operator to check router admission
doctor-step3-timeout-summary = zenoh session open timed out
doctor-step3-timeout-hint = router did not complete a session in { $secs }s — compare another mesh peer and inspect router health; this timeout alone does not prove a bad certificate
doctor-step4-ok-summary = portal returned { $n } org row(s)
doctor-step4-token-rejected-summary = portal rejected Bearer token
doctor-step4-token-rejected-hint = HTTP { $status } — ask your goatnet operator for a fresh goat-cli bundle, then run `goat profile rotate <new-bundle.cbor>` and rerun doctor
doctor-step4-request-failed-summary = portal request failed
doctor-step4-request-failed-hint = { $err } — the portal endpoint may be misconfigured or down; check `goat profile show`'s portal_endpoint, or run `goat status` for a broader read
doctor-step4-timeout-summary = portal request timed out
doctor-step4-timeout-hint = no response in { $secs }s — check { $endpoint } reachable from mesh
doctor-skipped-summary = skipped — prior step failed (re-run with --all to force)
doctor-skipped-no-endpoint-summary = skipped — profile.toml missing { $field }
doctor-skipped-no-profile-summary = skipped — no readable profile endpoints for this context
doctor-skipped-no-portal-token-summary = skipped — no readable portal token for this context

# ---------------------------------------------------------------------
# `goat --help` command-tree help (ADR 1024 / docs/design/goat-cli-i18n.md).
# These are the clap `about` / `long_about` / global-flag help strings,
# swapped onto the runtime command tree by `i18n::localize_command` after
# `Cli::command()` and before parse. clap's OWN chrome (the `Usage:` /
# `Options:` / `Commands:` headers and parser `error:` lines) stays
# English by ADR 1024 — only the prose goat authors localizes. Backticked
# `goat …`/`netbird …` commands, flag names (`--json`, `--all`), env vars
# ($GOAT_LANG), and paths are CONTRACT tokens: reproduce verbatim.
# ---------------------------------------------------------------------

# Root command about (short `-h`) + long_about (full `--help` paragraph).
help-about = Unified operator command-line for the goat data fabric.
help-long-about = goat — operator-facing CLI composing panoscope-core into one tool: profile init/reuse, ZTOL-aware reachability doctor, org portal browse, topic CRUD, pub/sub, stethoscope delegation, an interactive command composer, and substrate-cascade narration.

# Global flags (the Options: block on every help screen).
help-flag-profile-dir = Override the profile directory (default: $GOAT_PROFILE_DIR or ~/.goat).
help-flag-context = Override the active context (default: $GOAT_CONTEXT env, then current_context in config.toml, then "default").
help-flag-narrate = Show the substrate cascade a mutating command sets off.
help-flag-narrate-json = Emit the substrate cascade as NDJSON (one object per consequence) for incident logs + scripting. Implies --narrate.
help-flag-verbose = Enable verbose debug output (or set RUST_LOG=goat=debug).
help-flag-color = Color mode for terminal output (auto|always|never).
help-flag-lang = Language for human-facing prose (e.g. en, uk). Falls back to GOAT_LANG, then the OS locale, then English.

# Subcommand one-line descriptions (the Commands: list in `goat --help`).
# doctor additionally carries a long_about for `goat doctor --help`.
help-cmd-profile = Manage the local goat profile (mTLS cert + portal token + endpoint set).
help-cmd-context = Manage contexts (`ls` / `use` / `show` / `rm`).
help-cmd-init = First-run wizard: pick sandbox or real, prove it works, point at the tutor.
help-cmd-tutor = A self-paced, hands-on tour of the fabric (vimtutor-style; resumable).
help-cmd-docs = Browse the goat docs, or emit the full command reference (--markdown).
help-cmd-hello = Round-trip a hello through the fabric end-to-end (sandbox or real env).
help-cmd-doctor = Diagnose local setup, mesh client, router path, session, and portal.
help-cmd-doctor-long = Diagnose local readiness → router path → TLS material → mTLS session → portal. Each failure reports the observation and a next action.
help-cmd-vendor = Browse orgs registered in the portal.
help-cmd-topic = List, show, create, or delete topics.
help-cmd-pub = Publish a payload to a topic key.
help-cmd-sub = Subscribe to a topic key expression.
help-cmd-inspect = Delegate a diagnostic verdict request to stethoscope.
help-cmd-ui = Interactive merged picker: browse the declared ⊕ live roster and hop to the Diagnose / Tail lens on the selected target.
help-cmd-compose = Interactive command builder TUI.
help-cmd-narrate = Reflect over recent substrate activity from the audit-tail as a who-touched-what cascade (--since / --actor / --json).
help-cmd-completion = Print shell completion script to stdout.
help-cmd-explain = Interpret an error: a code (`E_OFF_MESH`, `GB-007`, `X-SSH-001`) or any error text, goat's own or a tool's words it carried — what happened, why, what to do, where to read.
help-cmd-mock = Publish synthetic samples to a topic. Default: trivial hello-world JSON at 1Hz. Pass `--format <.fds>` + `--message <Name>` to encode operator-supplied protobuf instead.
help-cmd-admin = Operator-only administration: org lifecycle, onboard/revoke/rotate, mesh-peer cut, device + operator-tier provisioning, LAN triage. Authorized by your operator credentials (server-side via portal/NetBird token scope); inert for non-admins — an unauthorized call changes nothing and fails with `E_NOT_ADMIN`.
help-cmd-status = Health synthesis: router port reachability + a real mTLS Zenoh session open + portal /api/health + local cert expiry + name, PQC and device state, one glyph line per check or one --json object.
help-cmd-services = Probe each hosted service in `ops/serve/services.yaml` and render one glyph line per service (or one --json object).
help-cmd-client = Wrap the local goat-clientd daemon via its JSON-RPC IPC socket (`~/.goat-client/goat-clientd.sock`): status / up / down / bundle import.
help-cmd-plugin = PATH-based extension discovery (kubectl-style). Any `goat-<name>` executable on PATH is invokable as `goat <name>`; `goat plugin list` enumerates installed plugins, `goat plugin which <name>` resolves the path.

# ---------------------------------------------------------------------
# `goat status` verdict summaries (ADR 1024 / docs/design/goat-cli-i18n.md).
# RENDER-BOUNDARY strings (the `tr2!` macro): the same summary feeds the
# human glyph line AND the serialized `--json` `summary` field, so the
# English half is pinned into `--json` (byte-stable across locales) while
# the human half localizes. Contract tokens reproduced verbatim:
# { $host } { $port } { $err } { $ms } { $secs } { $endpoint } { $tag }
# { $not_after } { $now } { $days_left }, plus the literals `/api/health`,
# `2xx`, `CERTIFICATE`, `unix=`, and the `d`/`ms`/`s` unit suffixes.
# ---------------------------------------------------------------------
status-ztol-malformed = router endpoint malformed: `{ $endpoint }`
# Renamed from `status-ztol-ok` (#1399): the old id's translations said
# "on mesh / reachable", the over-claim this line no longer makes. A new id
# means a stale catalog falls back to this English rather than rendering it.
status-ztol-tcp-ok = router port { $host }:{ $port } accepts TCP (port only — see session)
status-session-ok = zenoh mTLS session opened in { $ms }ms (router accepted this cert)
status-session-skipped = session not attempted (off mesh; see ztol)
status-session-material-missing = zenoh session not attempted: { $detail }
status-session-config-failed = zenoh session config failed: { $err }
status-session-failed = zenoh session did not open: { $err } — run `goat doctor`
status-session-timeout = zenoh session did not open in { $secs }s — the router port accepts TCP but no session completes (router wedged, cert rejected, or clock skew); run `goat doctor`
status-ztol-refused = TCP refused to router { $host }:{ $port }: { $err }
status-ztol-unreachable = off-mesh (router { $host }:{ $port } unreachable in { $ms }ms)
status-portal-ok = portal /api/health 2xx
status-portal-ok-no-key = portal /api/health 2xx (this context holds no portal key: liveness only)
status-portal-no-endpoint = portal not probed (this context names no portal)
status-portal-failed = portal /api/health failed: { $err }
status-portal-timeout = portal /api/health timed out in { $secs }s
status-cert-read-failed = cert read failed: { $err }
status-cert-pem-parse-failed = cert PEM parse failed: { $err }
status-cert-wrong-tag = first PEM block is { $tag }, want CERTIFICATE
status-cert-x509-parse-failed = x509 parse failed: { $err }
status-cert-expired = cert EXPIRED at unix={ $not_after }, now={ $now }
status-cert-valid = cert valid; expires in { $days_left }d (unix={ $not_after })
status-names-live = names live (mesh DNS answered)
status-names-ip-only = names not exercised (profile endpoints are IP literals)
status-names-snapshot = names as of signed snapshot serial { $serial }, age { $age } (mesh DNS unreachable)
status-names-snapshot-aging = names as of signed snapshot serial { $serial }, age { $age } — STALE (mesh DNS unreachable; fix goat-coredns, then names refresh automatically)
status-names-snapshot-daemon = names as of signed snapshot serial { $serial }, age { $age } — daemon store (mesh DNS unreachable)
status-names-snapshot-daemon-aging = names as of signed snapshot serial { $serial }, age { $age } — daemon store, STALE (mesh DNS unreachable; fix goat-coredns, then names refresh automatically)
status-names-failed = mesh DNS unreachable and snapshot fallback unavailable — { $reason }
status-pqc-on = wireguard PQC (rosenpass) ON — { $qr }/{ $connected } connected peers quantum-resistant
status-pqc-off = wireguard PQC (rosenpass) OFF — classical wireguard handshakes
status-pqc-unknown = wireguard PQC unknown — { $reason }
status-device-none = device-wide name protection OFF — no goat-clientd name store (goat commands still have fallback; enable per name-fallback-resolver.md §9.2)
status-device-broken = goat-clientd name store present (serial { $serial }) but its forwarder is NOT answering on { $addr } — daemon down or names subsystem wedged
status-device-active = device-wide name protection ACTIVE — goat-clientd fronting OS mesh DNS at { $addr } (store serial { $serial }, { $grade })
status-device-not-fronted = goat-clientd forwarder up at { $addr } (store serial { $serial }, { $grade }) — OS not fronted; browsers use direct mesh DNS
status-device-fronting-unknown = goat-clientd forwarder up at { $addr } (store serial { $serial }, { $grade }) — OS fronting state undetectable on this platform
status-names-observed = names NONCANONICAL (ad hoc): observed record, age { $age } — not in signed snapshot (mesh DNS unreachable; codify this record)
status-names-selfclaim = names via verified SELF-CLAIM, age { $age } — signed by the name's own anchor (mesh DNS unreachable)
doctor-names-note = via snapshot serial { $serial }, age { $age }
doctor-names-selfclaim-note = via verified SELF-CLAIM, age { $age } — signed by the name's own anchor
doctor-names-selfclaim-hint = mesh DNS is unreachable — this step resolved via a verified SELF-CLAIM (age { $age }): the name's own anchor signed this binding and the cached snapshot's peer_bindings authorize it. Signed but not live; fix mesh DNS to clear this.
doctor-names-observed-note = via OBSERVED (ad hoc) record, age { $age } — noncanonical
doctor-names-observed-hint = mesh DNS is unreachable — this step resolved via a NONCANONICAL observed record (age { $age }): the name is live-only, never codified in the site registry. Codify it (group_vars registry → generator → re-sign) or it ages out; fix mesh DNS to clear this.
doctor-names-hint = mesh DNS is unreachable — this step resolved via the signed name snapshot (serial { $serial }, age { $age }). The fabric stays reachable on fallback names; fix mesh DNS (goat-coredns on the mgmt host) to clear this.

# All-green flourish (docs/design/goat-cli-onboarding-and-delight.md) —
# appended to `goat doctor` / `goat status` ONLY on full success, pretty
# mode only, suppressed by GOAT_NO_FUN. Human prose; the 🐐 is a verbatim
# contract token (reproduce it as-is in every translation).
flourish-all-green = 🐐 all green — fabric nominal

# ---------------------------------------------------------------------
# `goat inspect` pretty-render labels (ADR 1024). Plain human prose — the
# pretty path is human-only; `--json` echoes stethoscope's raw event
# bytes untouched. The words `finding` / `remedy` localize; the bracketed
# values { $severity } { $label } { $detail } { $title } are
# stethoscope-supplied content reproduced verbatim, and the `()` / `:` /
# `—` punctuation is contract structure.
# ---------------------------------------------------------------------
inspect-finding-label = finding ({ $severity }): { $label } — { $detail }
inspect-remedy-label = remedy: { $title } — { $detail }
inspect-no-outcomes = (no outcomes — target matched no checks)

# ---------------------------------------------------------------------
# `--narrate` inline substrate-cascade lines (ADR 1024). Pretty path
# only; the `--narrate --json` path emits structured fields and stays
# English by rule. Contract tokens reproduced verbatim: { $method }
# { $path } { $http_status } { $outcome } { $key } { $bytes } { $check_id }
# { $verdict } { $summary } { $n }; the `→` glyph, backticks, and parens
# are structure; `Zenoh` and `stethoscope` are product names (verbatim).
# narrator-settled selects a plural form on { $n } — a translator uses the
# CLDR plural categories for their language ([one]/[few]/[many]/[other] as
# applicable); the `*[other]` default variant is mandatory.
# ---------------------------------------------------------------------
narrator-portal-confirmed = { $method } { $path } → { $http_status } { $outcome } (portal-confirmed)
narrator-zenoh-put = Zenoh put accepted on `{ $key }` ({ $bytes } bytes)
narrator-stethoscope = stethoscope: { $check_id } → { $verdict } ({ $summary })
narrator-settled = cascade settled — { $summary } ({ $n } { $n ->
        [one] consequence
       *[other] consequences
    })

# ---------------------------------------------------------------------
# `goat admin storm brake` — the manual control-plane storm brake (ADR
# 1081 Layer 4). Human prose only; step-kind labels (FLUSH/TIGHTEN/SHED/
# ANNOUNCE), env var names, paths, URLs, and backticked commands are
# CONTRACT tokens reproduced verbatim. { $user } { $host } { $context }
# { $container } { $unit } { $window } { $site } { $actor } { $ok }
# { $failed } { $skipped } { $reason } are interpolations passed through.
# ---------------------------------------------------------------------
storm-brake-plan-header = Storm brake plan (ADR 1081 Layer 4 — manual):
storm-brake-plan-target = mgmt host { $user }@{ $host }  (context { $context })
storm-brake-step-flush = restart signal container `{ $container }` (drops zombie streams)
storm-brake-step-tighten = clamp L2 limiter on `{ $container }` (halve rate-limit, raise neg-cache TTL) — { $window } min, operator-reverted; skipped if L2 not deployed
storm-brake-step-shed = pause non-essential publisher unit `{ $unit }`
storm-brake-shed-none = (no shed units declared — set storm_brake_shed_units in profile.toml or pass --shed-unit)
storm-brake-announce-note = emit one OperatorActionV1 to the audit trail with per-step outcomes
storm-brake-dryrun-note = dry run — nothing was executed.
storm-brake-triggers-header = Fire when the relay-health host-net band shows:
storm-brake-trigger-futility = signaling futility ratio >= 0.5 at >= 100 msgs/min (control.signal_notfound_60s / signal_msgs_60s)
storm-brake-trigger-loss = uplink loss >= 5% sustained 10 min (wan.loss_pct)
storm-brake-triggers-band-url = host band: relay-health.{ $site }.<zone> (see docs/design/relay-health-dashboard.md)
storm-brake-not-cure = Note: a signal flush alone does NOT restore the mesh. Layers 1-2 (client backoff + signal shedding) are the cure; the brake buys recovery time.
storm-brake-confirm = Fire the storm brake against { $user }@{ $host }? Restarts the signal container and pauses declared publishers. [y/N]
storm-brake-aborted = storm brake aborted (no confirmation)
storm-brake-executing = firing storm brake against { $user }@{ $host } ...
storm-brake-outcome-ok = ok
storm-brake-outcome-failed = failed
storm-brake-outcome-skipped = skipped
storm-brake-summary = storm brake complete: { $ok } ok, { $failed } failed, { $skipped } skipped
storm-brake-flush-failed = FLUSH failed — the signal container did not restart, so mesh recovery is not expected: { $reason }
storm-brake-mgmt-unresolved = mgmt host not resolved — pass --mgmt-host, set mgmt_ssh_host in the context profile.toml, or export GOAT_MGMT_SSH_HOST
storm-brake-audit-note = recorded operator action to the audit trail (actor { $actor })
