# Critical and high code scanning review

The baseline is goatcore main at `96ad9d47f30c67f14943b0c5dfdcf8065175bd60` on 4 November 2026. GitHub reported 104 open alerts in scope: 12 critical and 92 high. The adjacent [triage ledger](code-scanning-2026-11-04.json) records the source, classification and reasoning for each alert.

Eight alerts have code fixes. The remaining 96 need contextual dismissal, including all 12 critical findings. No query is disabled and no production crypto buffer is replaced merely to silence a scanner.

## Changes

- Portal and SNITCH browser clients exchange their API key for an opaque session cookie. The API key remains in server memory. Cookies are host-only, HttpOnly and SameSite Strict, and Secure outside loopback development. Sessions expire after eight hours. Cookie-authenticated writes require a matching Origin; the underlying key and role are revalidated on every request. An explicit invalid Authorization header cannot fall back to a cookie.
- Old Web Storage keys are removed without reuse. Downloads, uploads, streams and identity hydration use the session. Swagger authorization no longer persists across reloads. The existing bearer API remains available to command-line clients.
- Portal cache responses vary on Cookie as well as Authorization and content encoding.
- HTTPS service probes validate certificate chains and hostnames. A catalogue check can set `ca_file` to a PEM trust bundle for a private service; absent or invalid configured trust fails the check.
- Python TLS inspection contexts have an explicit TLS 1.2 minimum. Related application tests now verify HTTPS, including the fixture that sends the management token; private roots can use `REQUESTS_CA_BUNDLE`.
- 1Password item creation and editing receive JSON through stdin. No plaintext template file is written, and raw subprocess stderr is excluded from the status report.

These changes require newly built portal, SNITCH and smoketest images to affect deployed services. Browser sessions are per serving process: restart or failover requires sign-in again, and concurrent replicas need session affinity. The static frontend and corresponding API must be delivered together.

## Contextual findings

The critical crypto alerts concern buffers overwritten by a CSPRNG, authenticated plaintext or successful HKDF expansion before use, plus public test vectors and topic-path tests. The mailer rejects recipient line breaks and sanitizes all header values; existing injection regressions pass.

Most Rust logging paths display routing expressions, display-mode flags, public anchors or certificate timestamps. Six alerts concern deliberate output from privileged credential-issuance or handoff-rendering commands. Those commands return sensitive material as their requested result; they are not background service logging. This distinction is explicitly recorded in the ledger so an output-policy review can revisit it.

The two SPIFFE clients replace hostname checks with certificate-chain and allowed-URI verification. Three diagnostic probes deliberately inspect untrusted certificates or measure reachability without sending credentials. The service-health probe is treated separately and hardened.

Other findings concern existing path containment, high-entropy token hashing, MIME attachment bytes crossing imprecisely modeled ResponseWriter implementations, fixture extraction, and status reports containing names and counts rather than secret values.

## Verification

Run the database-backed tests with a disposable PostgreSQL database named by `TEST_PG_DSN`. Each portal test creates and removes its own schema. The SNITCH browser-auth test also creates a separate temporary schema. No production database is needed.

```zsh
(cd ops/goat-id-console && go test -race ./internal/mailer ./internal/spiffemtls)
(cd ops/onboard-runner && go test -race ./...)
(cd ops/goat-zenoh-portal && go test -race ./internal/api ./web)
(cd ops/goat-zenoh-portal && go test -race ./internal/api -run 'Test(Browser|Cache|OpenAPI|ResolvePrincipal)' -timeout 3m)
(cd ops/snitch && go test -race ./internal/auth ./internal/server)
(cd ops/smoketest && go test -race ./internal/probe ./internal/catalogue ./internal/server)
cargo test --locked --manifest-path ops/grant-crypto/sdk/Cargo.toml -p grant-crypto -j 2
python3 -m unittest discover -s ops/scripts -p test_netbird_service_identity_storage.py
node --test ops/goat-zenoh-portal/web/e2e/request-errors.test.mjs
node ops/ory-consent/tests/double-submit-guard.test.js
(cd ops/goat-zenoh-portal/web/e2e && npm ci --ignore-scripts)
(cd ops/goat-zenoh-portal/web/e2e && CI=true npx playwright test tests/browser-session.spec.ts tests/controls.spec.ts tests/navigation.spec.ts tests/inspector-stream.spec.ts tests/topic-form.spec.ts --workers=2 --reporter=list)
```

The crypto suite passed 35 tests, with two fixture/interoperability tests intentionally ignored. The focused browser suite passed all 63 tests. Go race tests passed for the affected modules, including real PostgreSQL-backed portal tests and SNITCH role/revocation tests. The final targeted portal run also covers the cookie cache and API document changes.

The full browser baseline has seven mobile overflow failures; the changed tree is compared against that baseline. The validation self-test has two existing fixture-name mismatches, reproduced without these changes. Neither is attributed to this security patch.

Local CodeQL security-extended analysis of the changed JavaScript surfaces produced zero results. Python analysis confirmed removal of the targeted insecure-protocol and plaintext-template-file findings; names-only diagnostic reports remain for contextual triage. Go and Rust findings additionally rely on the downloaded GitHub data-flow traces, source review and the tests above; a local full Go/Rust CodeQL rescan was not performed.

## Delivery and closure

Contextual dismissals must retain the ledger evidence. Code fixes close the eight affected main-branch alerts only after the patch is merged and the corresponding main-branch scan runs. The final GitHub inventory, rather than a passing local test or PR scan alone, establishes closure.

These public repositories are generated exports under ADR 1215. Replay the changes into the maintained source/export workflow before its next publication so regeneration preserves them. No live deployment is included in this change.

## CI follow-up

The initial PR run exposed a SNITCH lint regression: test requests needed explicit contexts, and the security linter could not infer the conditional Secure flag. Tests now use NewRequestWithContext. The two cookie statements carry a narrow G124 exception for loopback development, with tests covering external hosts and lookalike hostnames; HttpOnly and SameSite Strict remain unconditional. The exact CI linter, golangci-lint v2.12.1, reports zero issues.

GitHub default setup is configured for the repository and already publishes the authoritative code-scanning results. GitHub rejects advanced-setup uploads while default setup is enabled. The additional security-and-quality workflow still runs its queries and saves SARIF artifacts, without submitting a competing upload. Default setup remains enabled.

The Trivy job cannot access the two curated consistency images. Neither image appears among the 43 organization container packages visible to the authenticated operator, so authentication alone does not establish a fix. The workflow retains its original minimal permissions. The images must be published or made accessible before those advisory rows can scan successfully; no image is removed from coverage or marked clean because it could not be scanned.
