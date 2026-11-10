# goat-zenoh-edge

One dedicated Zenoh router per federated partner, on the peering edge host
(ADR 1157). The play is `ops/ansible/peering-zenoh-edge.yml`, phase 12 of
`goatbuilder federate` and `ops/scripts/peering-standup.sh`. Every variable,
with the reason for it, is in [defaults/main.yml](defaults/main.yml). Design:
[cross-sovereign-federation.md](../../../../docs/design/cross-sovereign-federation.md).

The role takes its values from two places. `goat federate apply` renders the
agreement half (partner id and name, key expressions, transport addresses) into
`<fed_work>/applied/ops/ansible/group_vars/<edge group>/`. The deployment half
(image, our CA, core endpoints, trust mode) goes in the deployment's own
group_vars. The role refuses to run with any required value empty.

## Peering certificates

The edge reads five files: two leaves with their keys, which phase 8
(`ops/federation/standup/mint-peering-leaves.sh`) mints into `<fed_work>/pki/`,
and the partner's CA, which phase 5 places at `<fed_work>/theirs-ca.pem`.

With `goat_zenoh_edge_install_certs: true` the role installs them itself, from
the run's working directory on the controller, into the per-peering directory
`goat_zenoh_edge_cert_dir`. With it false (the default for now), a person places
the files on the edge and sets the five path variables to them.

| Variable | Default | What |
|---|---|---|
| `goat_zenoh_edge_install_certs` | `false` | install the five files from the run's working directory |
| `goat_zenoh_edge_cert_dir` | `/etc/goat-zenoh-edge-<partner>/tls` | the per-peering directory on the edge. `goatbuilder federate allocate` renders this same value for a group with the switch on; it is never the allocated `cert_dir`, which is `<work>/pki`, the source on the controller (`goat_zenoh_edge_cert_src_dir`) |
| `goat_zenoh_edge_cert_src_dir` | `<fed_work>/pki` | the leaves, on the controller |
| `goat_zenoh_edge_partner_ca_src` | `<fed_work>/theirs-ca.pem` | the partner's CA, on the controller |
| `goat_zenoh_edge_agreement_src` | `<fed_work>/agreement.json` | the agreement that names both CA fingerprints |
| `goat_zenoh_edge_partner_ca_fingerprint` | empty (read from the agreement) | a pin; a value that differs from the agreement is refused |
| `goat_zenoh_edge_cert_min_valid_seconds` | `0` | refuse a leaf or the partner CA with less remaining lifetime (0 refuses only an expired one) |
| `goat_zenoh_edge_cert_owner`, `_group` | `root` | owner of the installed files |

The driver passes `-e fed_work=<work>`, as it does for every peering play. When
installing, the five path variables (`goat_zenoh_edge_outward_cert`, `_key`,
`goat_zenoh_edge_inward_cert`, `_key`, `goat_zenoh_edge_partner_ca_path`) are
derived from `goat_zenoh_edge_cert_dir`; a deployment that still sets one of them
to another path is refused with the name of the setting to remove.

Before the role writes anything on the edge, `tasks/certificates-check.yml`
runs `files/check-peering-certs.py` on the controller, and the role stops on any
failure. The script reads each file once (no symlink, regular files only) into a
private temporary directory and checks those copies:

- the agreement (`<fed_work>/agreement.json`, trusted as phase 6 verified it)
  names exactly two distinct parties, one of them the partner; a malformed one
  is a refusal, not a traceback;
- each leaf file holds exactly one certificate; each key belongs to its
  certificate (public keys compared) and has mode 0600;
- each leaf carries the identity the agreement derives, as phase 8 mints it:
  outward subject `CN=fed.<us>.<them>.01`, inward `CN=fed.<them>.<us>.01`, the
  same DNS SAN, `CA:FALSE`, and is neither CA certificate itself;
- each leaf chains to our CA, meaning the certificate in
  `goat_zenoh_edge_our_ca_path` (read from the edge) whose SHA-256 the agreement
  names for us, for TLS client and server use, and is valid for at least
  `goat_zenoh_edge_cert_min_valid_seconds`;
- the partner CA file holds exactly one certificate, a CA (`CA:TRUE`) valid as
  long, whose SHA-256 over DER is the `ca_fingerprint` the agreement names for
  the partner;
- `goat_zenoh_edge_cert_dir` is not held by another peering: its `.peering` file,
  when present, names this partner, and a directory without one is adopted only
  when it is empty or holds nothing but these five files with the checked bytes.

`tasks/certificates-install.yml` writes the files after the role's other checks.
A copy that transfers a file carries the checksum of the bytes checked, so a
source changed since the check fails it.

Keys are installed with mode 0600 and certificates with 0644, in a 0750
directory. A file whose content and mode are unchanged is left alone, so a re-run
restarts nothing; a changed file restarts the edge router once. Before the first
copy the role writes `.restart-pending` in the directory and removes it only
after the edge restarted (or at once when nothing changed): a run that stops in
between, or whose restart fails, leaves it, and the next run restarts the edge.

Test: `ops/ansible/roles/goat-zenoh-edge/tests/test-certificates.sh` mints
disposable CAs with openssl and runs the certificate tasks on localhost, covering
installation, an idempotent re-run, a `--check` run, a replaced leaf, each
refusal, two peerings on one edge, a failed restart and a half-done install, and
the whole role (`tests/main.yml`) for where the check and the install sit.
