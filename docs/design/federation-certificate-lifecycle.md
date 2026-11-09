# Federation certificate lifecycle

## Status

**Built:** phase-8 validation and reuse in `mint-peering-leaves.sh` and its Python
material helper, explicit replacement, output-directory locking, staged issuance
and regression tests using disposable CAs. **Not built:** a renewal scheduler,
service deployment, automatic acceptance refresh, or key distribution beyond the
edge's peering leaves and the republisher's leaf.
This change does not establish deployment or bilateral acceptance.

**Built:** installation of the two peering leaves and the partner CA on the edge by
the `goat-zenoh-edge` role, from the run's working directory, behind
`goat_zenoh_edge_install_certs` (default off), with its refusal checks and a
localhost test (see "Installing on the edge").

## Ordinary rerun

The shell helper continues to derive identities and addresses from the agreement.
It passes the requested outward, inward, republisher and optional core leaves to
`peering_leaf_material.py`. The verified agreement remains a prerequisite of the
existing driver; this helper does not replace package verification.

Validate all requested existing pairs before issuing any missing pair. Bind the own CA fingerprint to the agreement. Check the
exact subject CN, SAN set, client/server EKUs, key usage, non-CA constraints,
private-key/public-certificate match, key mode 0600, validity and chain under one
explicit own CA. Disable default trust locations for that verification. A changed
CA, identity, SAN, incomplete pair or invalid material stops the operation and
leaves certificate/key files unchanged. Existing valid material is reused without
rewriting it; neither a CA private key nor a new signature is needed for reuse.

The default minimum remaining lifetime is one day, configurable through
`FED_LEAF_MIN_VALID_SECONDS` or the helper's `--min-valid-seconds`. This is a refusal
threshold, not an automatic renewal trigger. `FED_LEAF_DAYS` controls new issuance.
Missing complete pairs require a matching own CA signing key. Private keys are
created under a private staging directory; fresh serials avoid mutating a shared
CA serial file.

## Explicit replacement

`--rotate` deliberately replaces every requested certificate/key pair. This is the
explicit reissuance path, including expiry. The operation requires the own CA key;
it is never selected automatically by the driver. Existing files are retained in
a private `.previous-*` directory. Key retention and retirement remain operator
responsibilities. Unrequested material, including a core pair when no core SAN is
provided, is left alone.

Reject symlink outputs and non-regular targets, including during rotation. A
non-blocking advisory lock serializes cooperating invocations using the same
output directory. The directory must be controlled by its operator; locking does
not defend against a privileged process replacing files outside this helper.

All candidate pairs must validate before installation. Individual file replacement
uses the same filesystem; ordinary installation errors attempt rollback. This is
not a crash-atomic multi-file transaction. A killed process or host failure during
installation can leave a mixed set. A `.rotation-pending` journal blocks further operations after an interrupted
replacement, including a mixed set of otherwise valid pairs. Restore the previous
set or complete the approved replacement using the retained private backup, verify
the complete set, then remove the marker under the operator recovery procedure. This PR does not promise unattended recovery.

## Installing on the edge

Status: WIP — gate: the federation e2e matrix passing with
`goat_zenoh_edge_install_certs: true` on both sides, then a run on two real
environments with no certificate file copied by hand.

The edge router reads five files: the outward and inward leaves with their keys,
from phase 8 (`<work>/pki/`), and the partner's CA, from phase 5
(`<work>/theirs-ca.pem`). Before this, a person copied them to the edge and
installed them by hand before phase 12. With `goat_zenoh_edge_install_certs: true`
the `goat-zenoh-edge` role installs them in phase 12 from the controller, as
`goat-fed-republisher` already installs its own leaf. The driver already passes
`fed_work` to every peering play, so no new driver value is needed.

Order, in `roles/goat-zenoh-edge/tasks/certificates.yml`:

1. The five path variables must equal `<cert_dir>/<file>` (the defaults derive
   them). A deployment that still sets one to another path is refused, naming it.
2. `goat_zenoh_edge_cert_dir` must be an absolute path at least two levels deep.
3. Our CA is read from the edge (`goat_zenoh_edge_our_ca_path`, read-only).
4. `files/check-peering-certs.py` runs on the controller and reports every failed
   check, not only the first: each key matches its certificate (public keys
   compared as DER) and has mode 0600; each leaf verifies against the certificate
   in our CA file whose SHA-256 is our party's `ca_fingerprint` in the agreement,
   for `sslserver` and `sslclient`, with default trust locations disabled (so an
   expired leaf fails too); the partner CA file holds exactly one certificate whose
   SHA-256 over DER is the partner's `ca_fingerprint`. A second certificate in the
   partner CA file is refused, not ignored: the edge would trust every certificate
   in it on the partner-facing listener without the agreement naming it.
5. The `.peering` file in `cert_dir`, when present, must name this partner.
6. Only then: the directory (0750), the `.peering` file, the certificates and
   partner CA (0644) and the keys (0600), each with the configured owner.

Steps 1 to 5 write nothing on the edge, so a refusal leaves it as it was. The
`copy` tasks compare content and mode, so an unchanged file is not rewritten and
the restart handler is notified only by a real change: a re-run restarts nothing,
and a rotated leaf restarts the edge once. The checks also run under `--check`.

Several peerings on one edge each have their own `cert_dir`. The default is
`/etc/goat-zenoh-edge-<partner>/tls`; the driver passes the allocated
`PeeringValues.cert_dir` once allocation lands. The `.peering` marker makes a
directory that another peering already holds a refusal rather than an overwrite.

Not built: the driver's phase 10 text still tells the person to place the partner
CA, which stays correct while the switch defaults to off. Turning the default on,
and removing that text and the e2e harness's hand install of `partner-ca.pem`, is
a separate later change. Our CA itself (`goat_zenoh_edge_our_ca_path`) and the
core's own certificate are still placed by the deployment.

`ops/ansible/roles/goat-zenoh-edge/tests/test-certificates.sh` mints disposable
CAs with openssl and runs the tasks on localhost: installation with the modes
above, an idempotent re-run with no restart, a `--check` run, a replaced leaf with
one restart, a second directory, and a refusal with an unwritten directory for a
mismatched key, a leaf from another CA, a wrong partner fingerprint, a two-
certificate partner CA file, a pinned fingerprint that differs from the agreement,
a key readable by others, a missing leaf, a directory another peering holds, a
path set by hand and a relative directory. CI runs it in
`.github/workflows/ansible-syntax-check.yml`.

## Boundaries and tests

Do not treat this operation as deployment or rotate running services implicitly.
Leaf replacement still needs a reviewed distribution/restart plan. Other phases
may publish artifacts or produce fresh evidence; this fix does not make the entire
standup workflow idempotent.

`python3 -m unittest discover -s tests -p test_peering_certificate_reuse.py`
exercises real OpenSSL issuance and reruns against synthetic CAs, including byte,
mtime and mode preservation, missing/partial pairs, wrong keys, same-name foreign
CAs, changed identities/SANs/EKUs, validity, explicit rotation, backup retention,
concurrent invocations and unsafe target handling. Existing federation end-to-end
CI exercises the driver with real ephemeral infrastructure; it remains required
and is not replaced by these filesystem/PKI tests.
