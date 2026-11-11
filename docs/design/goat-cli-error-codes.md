# goat-cli — error code namespace

Source of truth for the stable error codes emitted by `goat`. Every
code lands here with one-line meaning + remediation. The Phase E
`goat explain <code>` subcommand reads from this doc; scripts grep on
the codes; the code strings are stable across releases.

Adding a code: append a row below, a `GoatError` variant in
`crates/goat-cli/src/errors.rs`, and the entry in `ops/errors/catalog.yaml`
(the catalog `goat explain` reads, shared with goatbuilder — ADR 1199,
[error-interpretation.md](error-interpretation.md)). All three land in the
same commit; `ops/ci/check-error-catalog.py` refuses a drift between them.

## Error codes

| Code | Meaning | Exit code | Remediation |
|---|---|---|---|
| `E_PROFILE_MISSING` | `goat <subcommand>` invoked before `goat profile init`, or the referenced context lacks `profile.toml` / cert / key. | 3 | `goat profile init <bundle.cbor>` or pass `--context <name>` if the bundle is for a non-default context. |
| `E_PROFILE_DENIED` | The profile exists but this user cannot read it (permission denied — e.g. a root-owned 0700 profile dir probed without sudo). | 3 | `sudo chown -R $USER ~/.goat`, or re-run as the owning user with `sudo -E goat …`. Bare `sudo` resets `$HOME`, so it probes `/root/.goat` and reports a fresh missing profile. Do NOT re-init: the profile is there. |
| `E_BUNDLE_INVALID` | Bundle format, signature, or expiry verification failed at `goat profile init`. | 3 | Read the `help:` line — it distinguishes the three. Malformed CBOR means a bad download. A CA this machine does not trust means `goat ca add <root.pem>` (goat compiles in no root; `goat ca ls` shows what is trusted). Expiry means re-issue via operator onboarding. |
| `E_OFF_MESH` | The local mesh client reports no connected tunnel. | 5 | Inspect `netbird status -d` or `goat client diagnostics`; if already enrolled, run `goat heal` and rerun `goat doctor`. A reachable router TCP port alone does not prove mesh enrollment. |
| `E_CLOCK_SKEW` | The device clock differs from the configured management server's HTTPS Date by more than 60 seconds. | 6 | Synchronize the device clock and rerun `goat doctor`. |
| `E_ROUTER_UNREACHABLE` | The configured router TCP path or Zenoh session timed out or could not be reached. | 5 | Check tunnel state, DNS, route, and router service. Compare from another enrolled peer before changing enrollment. |
| `E_CERT_EXPIRED` | The local mTLS certificate is outside its validity interval. | 6 | Run `goat profile show`. Obtain a fresh issued bundle and run `goat profile rotate <new-bundle.cbor>`. |
| `E_DOCTOR_INCONCLUSIVE` | Doctor observed a local TLS/configuration failure or session rejection without enough evidence to identify one cause. | 1 | Follow the detailed hint in the doctor step; if the local certificate is valid, provide the step output to the goatnet operator for router admission diagnosis. |
| `E_PORTAL_UNREACHABLE` | Portal REST API not reachable for network reasons (DNS / TCP / TLS). | 4 | Check the portal host's mesh reachability. If reachable from another peer, the issue is local — `goat doctor --all` walks the full chain. |
| `E_PORTAL_BAD_RESPONSE` | The portal answered (2xx) but the body did not deserialize into the shape this `goat` build expects — a response-contract mismatch, not an unreachable portal. Distinct from `E_PORTAL_UNREACHABLE` so a healthy portal is never misdiagnosed as "down". | 4 | Almost always a portal/CLI version skew. Update `goat` (re-install from `get.<site>.*`; `goat doctor` flags if you're behind the served version). If you're already current, the portal deployed ahead of the CLI — report the endpoint + `goat --version`. |
| `E_PORTAL_NOT_FOUND` | The portal is up and answered HTTP 404 for the request path shown: what the command named (org, topic, schema, machine, person) is not registered there (#2087). | 7 | Check the name; `goat vendor ls` / `goat topic ls <org>/` list what exists; `goat context show` names the goatnet in use. |
| `E_PORTAL_RATE_LIMITED` | The portal is up and answered HTTP 429: this client is rate-limited. The hint carries the portal's `Retry-After` when sent. | 8 | Wait and retry; space out scripted calls. Retrying can succeed, which is why it has its own exit code. |
| `E_PORTAL_SERVER_ERROR` | The portal is up and answered HTTP 5xx for the request path shown: a fault on the portal's side. | 9 | Retry (after `Retry-After` if given); if it persists, a platform admin reads the portal's log on the management host. |
| `E_PORTAL_REFUSED` | The portal is up and answered a non-2xx status no command curates (400, 405, 410, 413, 422 …). | 4 | Usually a portal/CLI version skew: `goat doctor`, then update `goat`. |
| `E_TOKEN_INVALID` | Portal accepted the connection but rejected the Bearer token as invalid/expired (HTTP 401). | 4 | The token in `~/.goat/contexts/<name>/portal.token` may be expired or misissued. Re-run operator onboarding to get a new bundle, then `goat profile rotate <new-bundle.cbor>`. |
| `E_PORTAL_FORBIDDEN` | Portal returned HTTP 403 to the doctor API probe. The token was accepted but the identity lacks access. | 4 | Check the active context and request the relevant access grant from the goatnet operator. Rotating the same identity's token will not grant access. |
| `E_NOT_ADMIN` | An operator-only `goat admin` action was attempted without admin authority. Either the active profile carries no operator credentials at all (refused client-side, before anything is sent), or the portal / NetBird mgmt API returned HTTP 403 (valid credential, not authorized — a mutation 403 means nothing changed). Distinct from `E_TOKEN_INVALID` (401: bad/expired credential). | 4 | Confirm you're on your operator profile (`goat context use <name>`) and have sourced your operator environment (e.g. `GOAT_OPERATOR_HANDLE`, the NetBird admin token). If you still can't, your role isn't authorized — contact your goat administrator; re-onboarding the same identity won't grant it. `goat admin` is inert for non-admins by design. |
| `E_TOPIC_CONFLICT` | (Phase C) `goat topic create <topic>` attempted against a topic that already exists. | 1 | `goat topic show <topic>` to confirm shape; either pick a new name (consider versioning per `pubsub-best-practices.md`) or update the existing topic. |
| `E_MACHINE_UNRESOLVED` | A `goat machine` verb was given a handle or slot anchor that is not exactly one enrolled machine this profile can see (no match, or a handle under more than one custodian). Nothing was changed. | 1 | `goat machine ls` shows handle, custodian and anchor; pass the 32-hex anchor when the handle is ambiguous. |
| `E_MESH_PEER_UNRESOLVED` | The machine is enrolled but no NetBird peer carries its name (`machine-<handle>-<host>`, or `<handle>` when renamed by hand), or several do. Nothing was changed. | 1 | `goat admin peer ls` to find the peer; `goat machine mesh-key <machine>` when it never joined; rename a drifted peer back to `<handle>` or cut a stale duplicate with `goat admin peer cut <peer-id>`. |
| `E_NETBIRD_FAILED` | A NetBird management API call failed for a reason other than authorization (403 is `E_NOT_ADMIN`): token rejected (401), server unreachable, an unexpected status, or a peer record missing a setting the update must send back. Nothing was changed. | 4 | Export a current admin token as `<CONTEXT>_NB_API_TOKEN` (or `GOAT_NB_API_TOKEN`); `goat admin peer ls` proves URL and token; retry once the server answers. |
| `E_ACL_DECLARATION_MISSING` | `goat acl test` or `goat warrant` found no ACL declaration: no `--declaration`, no `GOAT_ACL_DECLARATION_DIR`, none in the active context, and no checkout carrying one. goat compiles in no deployment's declaration. | 1 | Pass `--declaration <dir>`, export `GOAT_ACL_DECLARATION_DIR`, set `acl_declaration_dir` in the context's profile.toml, run `goat context refresh` against a goatnet that serves `get.<site>/acl/`, or run inside a checkout carrying `ops/ansible/netbird_acl_*.yml`. |
| `E_MEMBERSHIP_VERSION_UNSUPPORTED` | A warrant-membership fragment or scope file parsed under its strict schema, or a generated baseline parsed under its compatibility schema, carries a numeric version this goat does not support. The typed error preserves the exact source filename and both version values. Fragment and scope files reject unknown fields before the version check; generated baselines deliberately permit unknown top-level fields. Other decode failures take precedence. Nothing was changed. | 1 | Update goat, check the named declaration's `version:`, and regenerate `_baseline.generated.yaml` from the reviewed operator manifests when that is the named file. Do not lower only the version number to bypass validation. |

## Exit codes (summary)

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | generic / unclassified |
| `2` | usage error (clap default) |
| `3` | profile missing or invalid |
| `4` | portal API unreachable, token rejected, or the portal refused the request (`E_PORTAL_REFUSED`) |
| `5` | Zenoh mesh / router unreachable |
| `6` | cert validity or clock failure |
| `7` | the portal answered HTTP 404: not registered (`E_PORTAL_NOT_FOUND`) |
| `8` | the portal answered HTTP 429: rate-limited, retry later (`E_PORTAL_RATE_LIMITED`) |
| `9` | the portal answered HTTP 5xx: a fault on its side (`E_PORTAL_SERVER_ERROR`) |
| `124` | timeout |
| `130` | SIGINT |

See `docs/design/goat-cli.md` §Error UX for the full rendering shape
(`error: <summary>` / `  help: <next-step>` / `  code: <code>`).
