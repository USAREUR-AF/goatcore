# Federation certificate lifecycle

## Status

**Built:** phase-8 validation and reuse in `mint-peering-leaves.sh` and its Python
material helper, explicit replacement, output-directory locking, staged issuance,
the CA key opened from the deployment's age-encrypted copy for one run, and
regression tests using disposable CAs. **Not built:** a renewal scheduler,
service deployment, automatic acceptance refresh, or key distribution beyond the
edge's peering leaves and the republisher's leaf.
This change does not establish deployment or bilateral acceptance.

**Built:** installation of the two peering leaves and the partner CA on the edge by
the `goat-zenoh-edge` role, from the run's working directory, behind
`goat_zenoh_edge_install_certs` (default off), with its refusal checks and a
localhost test (see "Installing on the edge").

## The CA key

Issuing a missing leaf, or rotating, needs our CA's private key. An explicit key
file always wins: `--ca-key` or `FED_CA_KEY`.

Opening the deployment's age-encrypted key is opt-in and off by default. With
`FED_CA_KEY_FROM_AGE=1` (or `--ca-key-from-age`; `goatbuilder federate` sets it
from `goat_federation_ca_key_from_age: true` in the edge group's vars), phase 8
opens `--ca-key-age` or `FED_CA_KEY_AGE` with `--age-identity` or
`FED_AGE_IDENTITY`. `goatbuilder federate` derives both, from
`goat_federation_ca_key_age` (else `<deploy.state_dir>/<site>-ca/ca.key.age`, the
portal role's `portal_ca_key_src`) and `goat_federation_age_identity` (else
`~/.config/age/keys.txt`). Without the opt-in, an age key that is present is not
opened, phase 8 says so on stderr, and a run that must issue refuses with the
helper's message, which names the opt-in.

The decrypted key goes to `ca-key.pem` in a fresh directory (mode 0700, the
file 0600) under `FED_CA_KEY_TMP` when it is set, else under memory-backed
`/dev/shm` where there is one, else under `$TMPDIR`, else `/tmp`. That directory
belongs to this run and is removed when the script exits, by any exit, including
an interrupt or termination. A kill the shell cannot trap (SIGKILL) leaves it
behind. An age key that does not open is reported on stderr, and the run goes on
without it. The report never repeats quoted text from age or anything shaped like
an age secret key, because age quotes the first line of a file it cannot read,
and that line is a secret when an identity file is named by mistake. A re-run
that reuses every leaf needs no key; one that must issue refuses with the
helper's message.

A later, separate change may make the opt-in the default.

Status: WIP — gate: on the federation test environment, phase 8 issues a fresh
peering's leaves with only the deployment's `ca.key.age`, its age identity and
the opt-in declared, and leaves no key file behind; without the opt-in nothing is
decrypted.

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

## Our CA beside the leaves

Once every requested pair is issued or reused, the helper places our CA, the
certificate whose fingerprint the agreement names for us, at
`<out>/our-ca.pem` (mode 0600, as the leaf keys: the edge container runs as root, the edge role reads it with become, and the republisher role copies it as the driver's user, who owns it), and rewrites it only when its content differs (a wider mode left by an earlier run is narrowed).
The rendered deployment halves (`goatbuilder federate allocate`, `deployment:
rendered`) name that file as the CA of the edge's inward endpoint
(`#root_ca_certificate_file=`) and as `goat_fed_republisher_core_ca_src`. Before
this, no phase wrote it: the edge's bind mount of a missing path left a directory
there, zenoh could not read the CA, discarded the endpoint and never dialled the
core, and phase 12 refused with "NO connection to any core endpoint". Something
other than a regular file at that path (such as that directory) is refused by
name before anything is issued. `--rotate` does not replace it.

Status: WIP — gate: nothing-hand-written on the federation test environment
passes phase 12 with the rendered edge half and no hand-placed `our-ca.pem`.

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

New pairs are minted into a private (0700) staging directory, `<out>/.mint-*`,
on the same filesystem, so installation is a rename. A run killed while minting
leaves that directory behind with the private keys it generated. The next run,
once it holds the lock and has found no `.rotation-pending` journal, removes every
`.mint-*` entry before it inspects anything: only the helper creates one, only
while it holds the lock, so an entry seen under the lock belongs to a run that is
no longer alive. A symlink of that name is unlinked, never followed. Installed
leaves, `our-ca.pem` and a rotation's `.previous-*` set are never removed. A run
resumed after a kill during minting leaves `<out>` in the same shape as a clean
run. Status: built, and measured by
`ops/federation/standup/test_peering_leaf_material_interrupted.py`, which kills a
real run while it is minting; the kill-resume scenario on the federation test
environment is not yet measured with this change.

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

Trust assumption: the agreement file in the controller's working directory
(`<work>/agreement.json`) is taken as the verified agreement. Phase 6 verified
its signatures; these checks do not verify them again. Anyone who can write the
controller's working directory can change what is checked, as they can change
the leaves themselves.

The checks and the writes are two task files, included at two points of
`roles/goat-zenoh-edge/tasks/main.yml`.

A driver that is not the edge host. Phase 8 writes `<work>/pki` on the machine
`goatbuilder federate` runs on, and the edge mounts each file by path on the edge
host. Before phase 12, the driver refuses (GB-272,
`crates/goatbuilder/src/federate/edge_delivery.rs`) when the edge group's vars
point the edge at files in `<work>/pki`, the inventory names the edge as another
machine (not `ansible_connection=local`, a loopback address or this machine's
name), and the switch is off: nothing would copy those files to the edge. With the
switch on, `goatbuilder federate allocate` renders our CA's path and the core
endpoint's `#root_ca_certificate_file=` as `/etc/goat-zenoh-edge-<partner>/tls/our-ca.pem`,
where the role installs it, instead of the controller's path. **Built**, unit and
stand-in tests only (`federate_edge_delivery.rs`); not yet run on the federation
test environment's operator-workspace scenario.

`tasks/certificates-check.yml` runs before the role's first write and only reads:

1. The five path variables must equal `<cert_dir>/<file>` (the defaults derive
   them). A deployment that still sets one to another path is refused, naming it.
2. `goat_zenoh_edge_cert_dir` must be an absolute path at least two levels deep,
   and the source directory is refused with a trailing `/` (the checksums are
   keyed by the exact source path).
3. Our CA. With the switch on, `goat_zenoh_edge_our_ca_path` defaults to
   `<cert_dir>/our-ca.pem`, and then the role installs our CA as well, from
   `goat_zenoh_edge_our_ca_src` on the controller (`<work>/pki/our-ca.pem`, where
   phase 8 places it). The source must be named `our-ca.pem` and hold
   certificates and nothing else: a file with any `PRIVATE KEY` block is
   refused, so our CA's key never reaches the edge. Any other
   `goat_zenoh_edge_our_ca_path` names a CA placed on the edge by hand, and our
   CA is read there, as before. **Built**: `tests/test-certificates.sh` installs
   it, refuses a source that holds a key or is not `our-ca.pem`, and checks the
   edge directory holds no CA key and the compose file mounts the installed file.
4. `files/check-peering-certs.py` runs on the controller. It reads each file once
   (no symlink, regular files only) into a private temporary directory, runs
   every `openssl` call on those copies, and hashes the same bytes. It reports
   every failed check, not only the first:
   - the agreement is a JSON object whose `parties` are exactly two objects with
     distinct `sovereign_id`s, one of them the partner, each with a
     `sha256:<hex>` `ca_fingerprint`; anything else is a `refused:` line;
   - each leaf file holds exactly one certificate; each key matches its
     certificate (public keys compared as DER) and has mode 0600;
   - each leaf carries the identity the agreement derives, as
     `mint-peering-leaves.sh` derives it: outward subject exactly
     `CN=fed.<us>.<them>.01`, inward exactly `CN=fed.<them>.<us>.01`, a DNS SAN
     equal to that CN, and `basicConstraints` `CA:FALSE`; a leaf whose DER is our
     CA or the partner CA is refused. A leaf minted for another peering, or our
     CA with its own key in a leaf's place, fails here;
   - each leaf verifies against the certificate in our CA file whose SHA-256 is
     our party's `ca_fingerprint`, for `sslserver` and `sslclient`, with default
     trust locations disabled and `-partial_chain` (that certificate may be an
     intermediate under a shared root; it is still the only anchor), and is valid
     for at least `goat_zenoh_edge_cert_min_valid_seconds` (`-checkend`);
   - the partner CA file holds exactly one certificate, with `basicConstraints`
     `CA:TRUE`, valid for at least the same time, whose SHA-256 over DER is the
     partner's `ca_fingerprint`. A second certificate in the partner CA file is
     refused, not ignored: the edge would trust every certificate in it on the
     partner-facing listener without the agreement naming it.

   On success it prints the SHA-1 of the bytes it checked for each source file.
5. The `.peering` file in `cert_dir`, when present, must name this partner. A
   directory without one is adopted only when it is empty, or when every file in
   it is one of the five this role installs with the same SHA-1 as the checked
   source. A hand-placed set from another peering, an older set, or a system
   directory is refused and left as it was.

`tasks/certificates-install.yml` runs after every other check in the role and
before the bundle trust mode reads the partner CA. It writes the directory
(0750), the `.peering` file, the certificates and partner CA (0644), our CA's
certificate when the role installs it (0644, written as the exact bytes the
check read) and the keys (0600), each with the configured owner. Each copy carries the checksum from step
4: a copy that transfers a source file changed after the check (a concurrent
`--rotate`, for example) fails instead of installing bytes nobody checked. A
destination already equal to its current source is not transferred, so that
comparison does not apply to it. The files are installed byte for byte as
checked; they are not re-encoded.

Restarts. Before the first copy the install writes `.restart-pending` in
`cert_dir`. When a file changed, or the file was already there from an earlier
run, it notifies the edge restart and then the handler that removes
`.restart-pending`; Ansible runs handlers in the order `handlers/main.yml`
defines them, and a failed restart fails the host, so the file is removed only
after a restart that succeeded. When nothing changed and nothing was pending,
the file is removed at once. A copy that fails, a later task that fails, a
failed restart, or an interrupted run leaves the file, and the next run restarts
the edge even when its own copies change nothing. Installing after the role's
other checks also means a later refusal does not follow a certificate change.

A refusal in the check file leaves the edge as it was, including a directory
already populated by an earlier install. The `copy` tasks compare content and
mode, so an unchanged file is not rewritten: a re-run restarts nothing, and a
rotated leaf restarts the edge once. The checks also run under `--check`. Key
files are copied with `diff: false`, so `--diff` does not print them.

Several peerings on one edge each have their own `cert_dir`. The default is
`/etc/goat-zenoh-edge-<partner>/tls`. When the edge group turns this switch on,
`goatbuilder federate allocate` renders the same per-peering directory,
`goat_zenoh_edge_cert_dir: /etc/goat-zenoh-edge-<with>/tls`, into the group's
`peering-values.yml`, and none of the leaf paths ([goatbuilder.md](goatbuilder.md),
"Per-peering files"). It does not pass the allocated `PeeringValues.cert_dir`:
that is `<work>/pki` on the machine running the driver, the source this role
reads the leaves from (`goat_zenoh_edge_cert_src_dir`), and it also holds the
core's and the republisher's leaves, so the unmarked-directory check below would
refuse installing into it whenever the driver runs on the edge host. Without
`allocate`, the default or a group_vars value applies. The `.peering` marker
makes a directory that another peering already holds a refusal rather than an
overwrite.

Not built: the driver's phase 10 text still tells the person to place the partner
CA, which stays correct while the switch defaults to off. Turning the default on,
and removing that text and the e2e harness's hand install of `partner-ca.pem`, is
a separate later change. Our CA itself (`goat_zenoh_edge_our_ca_path`) and the
core's own certificate are still placed by the deployment.

`ops/ansible/roles/goat-zenoh-edge/tests/test-certificates.sh` mints disposable
CAs with openssl and runs the task files on localhost (`tests/certificates.yml`)
and the whole role (`tests/main.yml`). Covered: installation with the modes
above, an idempotent re-run with no restart, a `--check` run, a replaced leaf with
one restart, our CA named as an intermediate inside a bundle; a refusal with an
unwritten directory for a mismatched key, a leaf from another CA, a wrong partner
fingerprint, a two-certificate partner CA file, a partner "CA" that is not a CA,
a pinned fingerprint that differs from the agreement, a key readable by others, a
missing leaf, a leaf file with two certificates, a leaf minted for another
peering, the inward identity in the outward place, a SAN that is not the CN, a
leaf with `CA:TRUE`, our CA and its key used as a leaf, a leaf or a partner CA
with less lifetime than the minimum, six malformed agreements (none a
traceback), a source directory with a trailing `/`, a directory another peering
holds, an unmarked directory holding other files or an older leaf, a path set by
hand and a relative directory; two peerings with two directories and two markers,
each refused the other's directory; five refusals against an installed directory,
which keeps its files, modes and restart count; a failed restart and a half-done
install, each leaving `.restart-pending` that the next run honors with one
restart. Through the whole role: with the switch on, the check runs and a later
refusal in `tasks/main.yml` leaves no certificate installed; bad material is
refused before the role's first write; with the switch off no certificate task
runs and no working directory is needed. Not covered by a test: an expired file
as such (the same `-checkend` refuses it with 0), a source file changing between
the check and the copy, and the role's real restart handler (the test stands in
for the compose restart). CI runs it in `.github/workflows/ansible-syntax-check.yml`.

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
