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
| `goat_zenoh_edge_cert_dir` | `/etc/goat-zenoh-edge-<partner>/tls` | the per-peering directory on the edge; the driver passes the allocated value |
| `goat_zenoh_edge_cert_src_dir` | `<fed_work>/pki` | the leaves, on the controller |
| `goat_zenoh_edge_partner_ca_src` | `<fed_work>/theirs-ca.pem` | the partner's CA, on the controller |
| `goat_zenoh_edge_agreement_src` | `<fed_work>/agreement.json` | the agreement that names both CA fingerprints |
| `goat_zenoh_edge_partner_ca_fingerprint` | empty (read from the agreement) | a pin; a value that differs from the agreement is refused |
| `goat_zenoh_edge_cert_min_valid_seconds` | `0` | refuse a leaf with less remaining lifetime (0 refuses only an expired leaf) |
| `goat_zenoh_edge_cert_owner`, `_group` | `root` | owner of the installed files |

The driver passes `-e fed_work=<work>`, as it does for every peering play. When
installing, the five path variables (`goat_zenoh_edge_outward_cert`, `_key`,
`goat_zenoh_edge_inward_cert`, `_key`, `goat_zenoh_edge_partner_ca_path`) are
derived from `goat_zenoh_edge_cert_dir`; a deployment that still sets one of them
to another path is refused with the name of the setting to remove.

Before anything is written on the edge, `files/check-peering-certs.py` checks on
the controller, and the role stops on any failure:

- each key belongs to its certificate (public keys compared) and has mode 0600;
- each leaf chains to our CA, meaning the certificate in
  `goat_zenoh_edge_our_ca_path` (read from the edge) whose SHA-256 the agreement
  names for us, for TLS client and server use, and has not expired;
- the partner CA file holds exactly one certificate, and its SHA-256 over DER is
  the `ca_fingerprint` the agreement names for the partner;
- `goat_zenoh_edge_cert_dir` is not already held by another peering (the
  directory's `.peering` file names its owner).

Keys are installed with mode 0600 and certificates with 0644, in a 0750
directory. A file whose content and mode are unchanged is left alone, so a re-run
restarts nothing; a changed file restarts the edge router once.

Test: `ops/ansible/roles/goat-zenoh-edge/tests/test-certificates.sh` mints
disposable CAs with openssl and runs the certificate tasks on localhost, covering
installation, an idempotent re-run, a `--check` run, a replaced leaf, and each
refusal.
