# Federation certificate lifecycle

## Status

**Built:** phase-8 validation and reuse in `mint-peering-leaves.sh` and its Python
material helper, explicit replacement, output-directory locking, staged issuance
and regression tests using disposable CAs. **Not built:** a renewal scheduler,
service deployment, automatic key distribution or automatic acceptance refresh.
This change does not establish deployment or bilateral acceptance.

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
