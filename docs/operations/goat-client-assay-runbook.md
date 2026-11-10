# goat-client assay env — operator runbook

A disposable single-region goatnet for exercising **goat-client** against a real
netbird management plane, with the test peers structurally forced onto the relay
path. That last condition is the point: it is the shape our field wedges occur
in, and it is the one thing a laptop on a friendly LAN cannot reproduce.

- Terraform: `terraform/goat-client-assay.tf`
- Ansible: [`ops/ansible/goat-client-assay.yml`](../../ops/ansible/goat-client-assay.yml)
- Peer installer: `ops/goat-client-assay/install-goat-client.sh`
- group_vars: `ops/ansible/group_vars/goat_client_assay_hosts/vars.yml`
- Prior art it derives from: [cgnat-validation-runbook.md](cgnat-validation-runbook.md)

**This env is EPHEMERAL — destroy it the same day.** ~$2.70/day, and it runs a
netbird management plane with a seeded reusable setup key and a throwaway relay
secret. None of that should outlive the assay. §7 is mandatory, not optional.

**It touches no live goatnet.** Own VPC subnet, own security groups, own IAM
role, own DNS names, `Environment = goat-client-assay` tag, and every resource
gated behind `goat_client_assay_enabled` (default `false`).

---

## 0. What this env does and does not prove

**Proves:** that a `goat-clientd` build, running headless in `netbird-only`
mode, can import a signed bundle, enroll against a netbird management plane, and
establish and hold a peer connection *over the relay* — including whatever
happens to it over hours in that state.

**Does not prove:** anything about wg-cp0, the outer control plane, Zenoh, or
the GUI client. Does not reproduce a specific site's management-server version
unless you override the pin (see §6). Does not exercise multi-region signal
federation — that is what [cgnat-validation](cgnat-validation-runbook.md) is
for, and this env is deliberately single-region.

### How peers are forced onto the relay

Two mechanisms, and the distinction matters when you report a result.

**Structural (primary — but NOT sufficient alone; see the correction).**
Peers sit in a private subnet whose only egress is a NAT Gateway. AWS NATGW
is a symmetric NAT, so a peer's srflx candidate is useless to another peer.
The original design claimed the `assay_peer` security group opening **no**
peer-to-peer ingress would also kill the VPC-local host candidates.

> **CORRECTION (measured 2026-07-31, first live run):** the no-ingress SG
> does NOT kill the host-candidate path. WireGuard's simultaneous
> bidirectional UDP on matching 4-tuples (both sides send src=51820
> dst=peer:51820) reads to the stateful SG as return traffic to each side's
> own outbound flow, so the direct connection completes —
> `getPeerConnectivity` showed `path: "direct"` on every bridge. The
> host-level block below is therefore REQUIRED, not optional.

**Host-level block (required).** On each peer, drop all traffic from the
other peers' private addresses before (re)starting the daemon:

```
iptables -I INPUT 1 -s <other-peer-ip> -j DROP     # one rule per other peer
systemctl restart goat-clientd-headless
```

ICE still gathers and genuinely attempts the host candidates — they now
really fail — and the daemon falls back to the relay. Verified live: after
the block, every bridge reports `path: "relayed"`. run-assay.sh check E
fails loudly if any bridge is still `direct`, so a forgotten block cannot
silently produce direct-path results.

**Flag (optional, default off).** `GOAT_FORCE_RELAY=true` on the installer puts
`NB_FORCE_RELAY=true` in the daemon's environment, which makes netbird skip the
direct attempt entirely. Use it only to distinguish *"wedged while relayed"*
from *"wedged while probing for a direct path"*, and **say which mode produced
any result you report** — a wedge that reproduces only with the flag on is
weaker evidence.

STUN (coturn, on the assay host) is left **working** on purpose. Peers
relay-forced by the *absence* of STUN are in a different and less
interesting state than peers relay-forced by NAT.

---

## 1. Prerequisites

1. **Credentials in the environment** (direnv; see
   local-secrets-pattern.md):
   - `aws-vault` profile `legacy` (dev tree, `legacy.example`)
   - `CLOUDFLARE_API_TOKEN` — dev-tree token, for both DNS-01 challenges
   - `TF_VAR_aws_account_id` — the cross-account fuse; no default, fail-closed
2. **Two throwaway secrets you generate now**, not reused from any live env:
   ```
   export ASSAY_PG_PW=$(openssl rand -base64 24)
   export ASSAY_NB_ENCKEY=$(openssl rand -base64 32)
   export GOAT_CLIENT_ASSAY_RELAY_SECRET=$(openssl rand -base64 32)
   ```
   Keep them in the shell for the session. They die with the env.
3. **An age identity** at `~/.config/age/keys.txt` — the throwaway bundle-signing
   CA's private key is age-encrypted at rest.
4. **A Go toolchain** on the laptop, for `ca-init` / `bundle-create`.
5. **`jq`**, for the inventory generator.

### The headless `.deb` — read this before you start

There is **no published, versioned `goat-client-headless` release asset.** The
release workflow builds the headless package only in a smoke job, amd64 only,
and the cosign staging step explicitly excludes it from release assets. So:

**(a) Pull the CI artifact** — the current path. Artifact
`goat-client-headless-deb-amd64` from a `main` run on `goat-maintainer/goat-client`:

```
gh run download --repo goat-maintainer/goat-client <run-id> \
  -n goat-client-headless-deb-amd64 -D /tmp/assay
```

**(b) Build the commit under test** — if you are validating a branch that has
not merged, a CI artifact from `main` cannot represent it:

```
cd /path/to/goat-client
GOOS=linux GOARCH=amd64 go build -tags headless \
  -o dist/linux_amd64-headless/goat-clientd ./cmd/goat-clientd
VERSION=$(git rev-parse --short HEAD) GOARCH=amd64 \
  packaging/build-linux-pkg.sh deb-headless
```

**Do not match on a version string.** Main-push builds carry a placeholder
internal version of `0.0.1` — real versions come only from tags. The object
staged to S3 is deliberately named version-free
(`goat-client-headless_amd64.deb`), and the installer matches nothing on
version. The build under test is identified by its **commit SHA**, which you
record out-of-band for the evidence table in §5.3.

### Peers cannot fetch it themselves

The peers sit in a private subnet with no inbound path, and GitHub Actions
artifacts require auth. So the delivery channel is S3: the operator stages the
artifacts into the **existing** legacy ansible-SSM bucket
(`aws_s3_bucket.ansible_ssm` in `terraform/main.tf` — no new bucket) under the
`goat-client-assay/` prefix, and peers read them with `aws s3 cp` under their
instance profile. Terraform grants that role `s3:GetObject` on exactly that
prefix, plus a prefix-scoped `s3:ListBucket` so a permissions failure does not
masquerade as "Not Found".

> **That bucket expires every object after 1 day.**
> `aws_s3_bucket_lifecycle_configuration.ansible_ssm` applies
> `expiration { days = 1 }` with an empty filter — it is a transfer channel,
> not storage. **Stage the artifacts and run the assay the same day.** If you
> re-provision a peer roughly a day in, re-upload first; the installer will
> otherwise fail on a 404 that reads like a permissions problem. Never treat a
> staged bundle as the record of what you tested — keep the `.deb` and its
> commit SHA on the laptop.

---

## 2. Provision

```
cd /path/to/goat
```

Add to `terraform/terraform.auto.tfvars` (gitignored):

```
goat_client_assay_enabled    = true
goat_client_assay_peer_count = 3
# Optional: your laptop's public /32, only if you want to curl the mgmt API
# directly. SSM covers shell access without it, and leaving it empty keeps the
# management plane reachable ONLY from the assay's own NAT address.
# goat_client_assay_operator_cidr = "203.0.113.4/32"
```

Then apply, scoped so nothing else in the dev tree moves:

```
aws-vault exec legacy --no-session -- terraform -chdir=terraform apply \
  -target=aws_iam_role.assay_ssm \
  -target=aws_iam_role_policy_attachment.assay_ssm \
  -target=aws_iam_instance_profile.assay_ssm \
  -target=aws_subnet.assay_priv \
  -target=aws_eip.assay_natgw \
  -target=aws_nat_gateway.assay \
  -target=aws_route_table.assay_priv \
  -target=aws_route_table_association.assay_priv \
  -target=aws_security_group.assay_mgmt \
  -target=aws_security_group.assay_peer \
  -target=aws_instance.assay_mgmt \
  -target=aws_eip.assay_mgmt \
  -target=aws_instance.assay_peer \
  -target=cloudflare_record.assay_mgmt \
  -target=cloudflare_record.assay_relay
```

Capture outputs and regenerate the inventory:

```
aws-vault exec legacy --no-session -- terraform -chdir=terraform output | grep goat_client_assay
aws-vault exec legacy --no-session -- ops/ansible/inventory/gen-goat-client-assay.sh
```

**Wait for DNS before Ansible.** Both ACME challenges resolve the public names,
and Cloudflare propagation is fast but not instant:

```
dig +short assay.legacy.example
dig +short relay-assay.legacy.example
```

Both must return the mgmt host's EIP. Then bring the control plane up:

```
cd ops/ansible
NATGW_IP=$(aws-vault exec legacy --no-session -- \
  terraform -chdir=../../terraform output -raw goat_client_assay_natgw_public_ip)
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ../scripts/ansible-preflight.sh \
    -i inventory/goat-client-assay.ini \
    goat-client-assay.yml \
    --skip-tags archive_load \
    -e postgres_superuser_password="$ASSAY_PG_PW" \
    -e netbird_encryption_key="$ASSAY_NB_ENCKEY" \
    -e cf_api_token="$CLOUDFLARE_API_TOKEN" \
    -e cloudflare_api_token="$CLOUDFLARE_API_TOKEN" \
    -e "mesh_only_allowlist_extra_cidrs=[\"$NATGW_IP/32\"]" \
    -e assay_owner_email=person-69@example.org
```

For the F-269 token-self-heal check (run-assay.sh check D), also pass
`-e relay_credentials_ttl=2m` — the default 24h TTL means no token ever ages
out inside an assay session and check D reports SKIP-shaped behavior.

`mesh_only_allowlist_extra_cidrs` **is required here.** The imported mgmt
stack gates its websecure apex behind the Traefik `mesh-only-allowlist`
middleware (mesh CGNAT + docker-bridge sources only). In the goatnets that
template serves, enrollment arrives on a separate entrypoint; in this env the
peers enroll straight against the apex from the NAT gateway's address, and
without this entry every gRPC login 403s ("unexpected HTTP status code
received from server: 403") before a peer can ever become a mesh source.

`--skip-tags archive_load` **is required.** The imported goat-relay deploy has
an offline image-transport phase that runs on localhost and fails on a missing
local `docker save` archive; we pull from GHCR instead.

**Copy the setup key out of the Phase 4 output now.** The CLI prints the
plaintext exactly once and stores only its hash. A re-run replays without
reprinting it; if you lose it you must
`mgmt-host-bootstrap rotate-setup-key --id <sk_id> --idempotency-key <fresh>`.

Expected end state: signal `/healthz` 200, `https://assay.legacy.example/api/peers`
answering **401** with a valid public certificate, relay listening on 33073,
coturn on 3478, zero peers enrolled.

---

## 3. Mint the CA, the trust roots, and the bundles

A `netbird-only` peer takes its management URL and setup key **only** from a
signed CBOR bundle. Nothing in `config.toml` can supply them.

### 3.1 Throwaway signing CA

ECDSA **P-256** — the bundle verifier requires it, and an Ed25519 key fails to
load at all. (Several comments and one Ansible playbook in-tree still say
Ed25519; see the drift table in §6.)

```
mkdir -p ~/.goat/state/assay-ca
cd ops/enrollment
go run ./cmd/ca-init --mode dev \
  --id assay-ca-ecdsa-$(date +%Y-%m-%d) \
  --out ~/.goat/state/assay-ca/ca.key.age \
  --recipients "$(grep '# public key:' ~/.config/age/keys.txt | cut -d: -f2 | tr -d ' ')" \
  --allow-single-recipient \
  --age-identity ~/.config/age/keys.txt \
  > /tmp/assay-trust-roots.pem
```

`ca-init` prints the SPKI public key on **stdout** — that redirect *is* how you
produce `trust-roots.pem`. It also prefixes the id with `dev-`, so the value to
pass as `--ca-id` below is `dev-assay-ca-ecdsa-<date>`. The private key never
leaves your laptop and is not in the repo.

### 3.2 A dummy wg-cp0 relays file

`bundle-create` requires `--wg-cp0-address` and `--wg-cp0-relays` even for a
bundle that will only ever be used in `netbird-only` mode. The bundle *format*
marks every wg-cp0 field `omitempty` and the client's connect path is cleanly
mode-gated, so this material is inert here — it is purely a minter constraint.
Every field is validated, so the file must be well-formed even though it is
fiction:

```
cat > /tmp/assay-relays.json <<'EOF'
[{"name":"dummy","endpoint":"127.0.0.1:51821","address":"198.18.0.2",
  "pubkey":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}]
EOF
```

### 3.3 One bundle per peer

Per-peer bundles rather than one shared bundle: the device id composes into the
peer's advertised hostname, so distinct ids make the netbird peer list readable
during triage. A shared bundle would also work — goat-client enforces no
device-uniqueness — but attribution gets muddy exactly when you need it.

```
export ASSAY_SETUP_KEY='<plaintext from Phase 4>'
export ASSAY_CA_ID="dev-assay-ca-ecdsa-$(date +%Y-%m-%d)"

for i in 0 1 2; do
  go run ./cmd/bundle-create --mode dev \
    --ca-id "$ASSAY_CA_ID" \
    --ca-key ~/.goat/state/assay-ca/ca.key.age \
    --age-identity ~/.config/age/keys.txt \
    --site assay \
    --device-id "goat-assay-peer-$i" \
    --acl-groups ASSAY \
    --wg-cp0-address "198.18.0.10$i/24" \
    --wg-cp0-relays /tmp/assay-relays.json \
    --inner-mesh-mgmt-url "https://assay.legacy.example:443" \
    --inner-mesh-setup-key "$ASSAY_SETUP_KEY" \
    --ttl-days 7 --activation-days 7 \
    --out "/tmp/goat-assay-peer-$i.cbor"
done
```

`--activation-days` must be stated: its default (14) exceeds `--ttl-days 7`
and `bundle-create` refuses the combination.

`--inner-mesh-mgmt-url` **must** be `https://` — the minter rejects anything
else. That single constraint is why this env carries a Let's Encrypt cert on the
mgmt endpoint instead of the plaintext socat sidecar the CGNAT env used.

### 3.4 Stage the artifacts to S3

Into the existing ansible-SSM bucket, under the `goat-client-assay/` prefix the
peer instance profile is granted read on. No presigning — the peers use their
instance profile, which does not expire the way a presigned URL does, and you
will want to re-run the installer during triage.

```
SSM_BUCKET=$(aws-vault exec legacy --no-session -- \
  terraform -chdir=terraform output -raw ansible_ssm_bucket)

aws-vault exec legacy --no-session -- aws s3 cp \
  /tmp/assay/goat-client-headless_*_amd64.deb \
  "s3://$SSM_BUCKET/goat-client-assay/goat-client-headless_amd64.deb"

aws-vault exec legacy --no-session -- aws s3 cp \
  /tmp/assay-trust-roots.pem "s3://$SSM_BUCKET/goat-client-assay/trust-roots.pem"

for i in 0 1 2; do
  aws-vault exec legacy --no-session -- aws s3 cp \
    "/tmp/goat-assay-peer-$i.cbor" "s3://$SSM_BUCKET/goat-client-assay/goat-assay-peer-$i.cbor"
done
```

Note the destination `.deb` name drops the version — see the naming note in §1.
Confirm what landed, and remember the 1-day expiry:

```
aws-vault exec legacy --no-session -- aws s3 ls "s3://$SSM_BUCKET/goat-client-assay/"
```

---

## 4. Enroll the peers

One SSM command per peer. The installer is idempotent and retries, so re-running
a peer that partly converged is cheap and safe.

```
PEER_IDS=$(aws-vault exec legacy --no-session -- \
  terraform -chdir=terraform output -json goat_client_assay_peer_instance_ids | jq -r '.[]')
PREFIX="s3://$SSM_BUCKET/goat-client-assay"

i=0
for id in $PEER_IDS; do
  aws-vault exec legacy --no-session -- aws ssm send-command \
    --region <region> \
    --instance-ids "$id" \
    --document-name AWS-RunShellScript \
    --comment "goat-client-assay enroll peer $i" \
    --timeout-seconds 1200 \
    --parameters commands="[\"command -v aws >/dev/null || { export DEBIAN_FRONTEND=noninteractive; apt-get update -qq >/dev/null; apt-get install -y -qq unzip curl >/dev/null; curl -sS https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip -o /tmp/awscliv2.zip; unzip -q -o /tmp/awscliv2.zip -d /tmp; /tmp/aws/install >/dev/null; }; aws s3 cp '$PREFIX/install-goat-client.sh' /usr/local/sbin/install-goat-client.sh --no-progress; chmod 0755 /usr/local/sbin/install-goat-client.sh; GOAT_DEB_URL='$PREFIX/goat-client-headless_amd64.deb' GOAT_TRUST_ROOTS_URL='$PREFIX/trust-roots.pem' GOAT_BUNDLE_URL='$PREFIX/goat-assay-peer-$i.cbor' GOAT_MODE=netbird-only /usr/local/sbin/install-goat-client.sh\"]" \
    --query 'Command.CommandId' --output text
  i=$((i + 1))
done
```

The command self-heals the two cloud-init dependencies rather than assuming
them: on the 2026-07-31 first run, peer cloud-init had failed its package
phase (boot raced the NAT gateway), so the instances had neither the AWS CLI
nor `/usr/local/sbin/install-goat-client.sh` — and cloud-init does not
re-run. Fetching the installer here also picks up any re-staged version.

The installer takes `s3://` URIs and fetches them with `aws s3 cp` under the
instance profile, retrying throughout. An `https://` URL works too if you are
pulling from somewhere other than that bucket.

Fetch a result:

```
aws-vault exec legacy --no-session -- aws ssm get-command-invocation \
  --region <region> --command-id <cmd-id> --instance-id <peer-id> \
  --query '{Status:Status,Out:StandardOutputContent,Err:StandardErrorContent}' --output text
```

The installer exits **0** only once the inner mesh reports itself up. Exit **2**
means it timed out waiting — which may itself be the finding you came for; it
dumps `getStatus` and the last 80 journal lines before exiting.

To add the force-relay cross-check on one peer, append
`GOAT_FORCE_RELAY=true` to that peer's command and re-run it.

---

## 5. Verify, and what evidence to capture

### 5.1 The peers enrolled

From the mgmt host (`aws ssm start-session --target <mgmt-instance-id>`):

```
sudo /usr/local/bin/mgmt-host-bootstrap list-peers
```

Expect one row per peer. **A row alone is not success** — a peer that enrolls
and then shows no connection is the classic symptom of a mis-seeded account, and
it is exactly what this env's Phase 4 seeding is careful to avoid.

### 5.2 The peers are on the relay, not a direct path

This is the load-bearing check. On a peer:

```
echo '{"jsonrpc":"2.0","id":1,"method":"getInnerMeshStatus"}' \
  | sudo socat - UNIX-CONNECT:/run/goat-client/ipc.sock
echo '{"jsonrpc":"2.0","id":1,"method":"getPeerConnectivity"}' \
  | sudo socat - UNIX-CONNECT:/run/goat-client/ipc.sock
```

Read the **`innerMesh`** block, not the top-level `state`. In `netbird-only`
mode `state` reports `disconnected` **by design** — it describes the wg-cp0 leg,
which this mode does not run. Judging by `state` marks every healthy peer broken.

Corroborate from the relay side, on the mgmt host:

```
curl -s http://127.0.0.1:9098 | head -40      # goat-relay stats snapshot
docker logs --tail 100 goat-relay
```

Per-peer forwarded-byte counters climbing on the relay while peer connectivity
reports established is the positive result: traffic is transiting the relay
rather than a direct path.

### 5.3 Evidence to capture

Capture all of it before teardown — the env is gone afterwards and none of this
is recoverable.

| Evidence | Where from |
|---|---|
| Build under test | `dpkg-query -W goat-client-headless` on a peer, plus the goat-client commit SHA you built |
| Relay-path proof | `getPeerConnectivity` output + goat-relay per-peer counters, same timestamp |
| Whether force-relay was on | the `GOAT_FORCE_RELAY` value used per peer — state it explicitly |
| Daemon journal | `journalctl -u goat-clientd-headless.service --no-pager` per peer, full |
| Inner-mesh diagnostics | `getInnerMeshDiagnostics` over the IPC socket |
| Control-plane side | `docker logs management-a`, `docker logs signal-a`, signal `/metrics` |
| Wedge timing | wall-clock of first symptom vs peer start; a wedge with a latency is a different bug from one at handshake |
| Env identity | terraform outputs (NATGW IP, FQDNs, instance IDs) and the netbird management image digest actually running |

If you reproduce a wedge, write it up as a finding under `docs/findings/` and
re-check `docs/findings/INDEX.md` for a free F- number immediately before
pushing — those IDs race under concurrent sessions.

---

## 6. Divergences from the CGNAT prior art, and drift found

This env is derived from
`terraform/cgnat-validation.tf` and
[`ops/ansible/cgnat-validation.yml`](../../ops/ansible/cgnat-validation.yml).
Reused shapes: NATGW-as-symmetric-NAT, SSM instead of SSH, the
`default-for-az` subnet filter, explicit `depends_on` from peers to the NATGW
route. The deliberate divergences:

| Divergence | Why |
|---|---|
| Account/setup-key seeded via `mgmt-host-bootstrap`, not raw SQL | The CGNAT SQL self-documents as superseded by that CLI, and has since drifted behind it — it omits `setup_keys.auto_groups='[]'` and seeds **no default All→All accept policy**, without which "peers enroll but never connect (status shows 0/0)". Inheriting a bug that fabricates the exact symptom being hunted would poison every result. |
| No socat mgmt-expose sidecar; Traefik + Let's Encrypt on :443 | The bundle minter refuses a non-`https` management URL, so the plaintext-HTTP shortcut is simply unavailable to a goat-clientd peer. |
| Peers do **not** self-enroll from cloud-init | A netbird setup key can be baked into user_data before apply; a signed bundle cannot — it needs a key that does not exist until the mgmt plane is up. Enrollment is a post-Ansible SSM step. |
| Relay tier included | CGNAT was measuring signal federation and had no relay. The relay is the entire point here. |
| Ubuntu 24.04, not 22.04 | Matches the fleet OS baseline. No reason to reproduce a two-year-old base on a fresh throwaway. |
| Peer-facing ports allowlisted to the NATGW /32, not `0.0.0.0/0` | The peers are the only legitimate clients and they all egress from one address. CGNAT opened these to the world; no reason to repeat it. |
| Single region | Federation is out of scope; that is what the CGNAT env is for. |

### Drift and defects found while writing this

Each of these was verified in-tree, and each would have broken this env if
followed literally.

1. **The headless `.deb` is not a release asset.** Built only in a smoke job,
   `VERSION` hardcoded `0.0.1`, amd64 only, explicitly excluded from release
   assets by the cosign step. The upstream headless bringup doc assumes a
   published artifact for both amd64 and arm64; that assumption is not backed
   by the workflow. Hence §1(a).
2. **`ops/ansible/goat-client-install.yml` cannot be used here — CONFIRMED, and
   it is a live defect, not just an assay inconvenience.** Its hardcoded
   `goat_client_trust_anchor_pem` starts `MCowBQYDK2Vw`, the Ed25519 SPKI
   prefix, while goat-client's loader hard type-asserts P-256
   (`internal/trustanchor/anchor.go:114-121` `parseSPKI`, and
   `internal/bundle/trust.go` is `*ecdsa.PublicKey` throughout). A failed load
   is fatal at daemon start, so that playbook produces a permanently
   crash-looping unit on any host it touches. Two stale claims point the same
   wrong way and are worth fixing at the source: `goat-clientd`'s own
   `--trust-roots` flag help says "ECDSA P-256 or Ed25519", and
   `internal/trustanchor/doc.go` still describes the anchors as Ed25519. Only
   P-256 is accepted; the Ed25519 root was retired substrate-wide at Block 79.
3. **`bundle-verify` does not exist.** It is referenced from the goat-client
   troubleshooting doc and the headless bringup doc. `ops/enrollment/cmd/`
   contains `bundle-create`, `bundle-extract`, `ca-init`, `cert-sign`,
   `name-snapshot-sign`, `wg-cp0`, `wg-cp0-bundle-apply` — and no verifier.
4. **`GOAT_MODE=<x> apt install` does not work.** apt clears the environment
   before running maintainer scripts, so the mode must be written into
   `/etc/default/goat-client-headless` *before* installing. The headless bringup
   doc still advertises the broken form; the release workflow's own smoke test
   does it correctly. The installer here does it correctly.
5. **Release packages do not embed trust roots.** The bringup doc says they do.
   `goat-clientd` always reads the PEM from disk and exits 2 if it is missing,
   in every mode; the build-time embedded anchor set is wired only into the
   mobile SDKs.
6. ~~**The full (non-headless) systemd unit is broken.**~~ **RETRACTED
   2026-07-30 — this was a misreading and the unit is fine.** goat-client PR #90
   ("match goat-clientd flags") had already fixed it; what remains at
   `packaging/deb/systemd/goat-clientd.service:15` is a *comment recording that
   fix* ("has NO -bundle-dir / -ipc-socket / -log-file"), not a description of
   the ExecStart. Verified: both the deb and rpm units pass only `--bundle`,
   `--socket`, `--trust-roots`, `--config`, all of which `goat-clientd` defines.
   Kept rather than deleted as a reminder that a comment describing a past bug
   reads exactly like the bug when grepped.
7. **`bundle-create` requires wg-cp0 flags for an inner-mesh-only bundle.**
   A minter-only constraint; the format and the client are both fine with
   inner-mesh-only. Hence the dummy relays file in §3.2.
8. **`ca-init` comments and `ops/enrollment/README.md` still say Ed25519.**
   The code generates ECDSA P-256 and the verifier requires it. Stale comments.
9. (omitted)
10. **`netbird_mgmt_image` version envelope.** `mgmt-host-bootstrap` is
    CI-pinned against netbird-mgmt **0.68.3**, which is also the compose
    default — so leaving the pin unset (what this env does) keeps the DB seeding
    inside its verified envelope. A live goatnet may run a different
    `netbird-mgmt` image. **If the wedge you are chasing is suspected to be
    management-server-side rather than client-side, this env does not reproduce
    production by default** — override `netbird_mgmt_image` to the digest the
    affected goatnet is actually running, and re-verify that the Phase 4 seed
    still succeeds.
11. **Stale cost claim in the prior art.** `cgnat-validation.tf` says "EIPs
    in-use: free while attached". False since AWS began charging for all public
    IPv4 (Feb 2024). Immaterial at this scale, but not worth propagating.
12. **The ansible-SSM bucket expires everything after 1 day**, and nothing in
    the tree says so where an operator would look. It is a transfer channel,
    and the `community.aws.aws_ssm` connection plugin's objects are transient
    by nature — but it is now also the artifact-delivery path for this env, and
    a 404 there reads exactly like a permissions failure. Covered in §1 and in
    the installer's own error output.
13. **`AmazonSSMManagedInstanceCore` does not grant access to that bucket.** It
    covers only AWS's own `aws-ssm-*` / `amazon-ssm-*` buckets. Every existing
    env gets away with attaching it alone because the Ansible SSM connection
    plugin presigns on the controller, so no instance credentials are involved.
    Fetching with `aws s3 cp` under the instance profile therefore needs the
    explicit prefix-scoped grant this env adds — worth knowing before assuming
    an instance with the SSM role can read the bucket.
14. **Ubuntu's cloud image ships no AWS CLI.** Peers install it via cloud-init,
    with a fallback to the official v2 installer in the script for manual
    re-runs on a host where cloud-init did not complete.

### Defects found on the first live run (2026-07-31)

Each was hit live, in this order, and each is now either fixed in-tree or
carried in the section it belongs to.

15. **The goat-relay role's own defaults crash-looped the relay.** Default
    image pin `sha-82acd47` predates `--stats-addr` (#1349), while the role
    passes it by default (`goat_relay_stats_addr` set) — unknown-flag →
    usage → exit, on loop. Fixed: role default bumped to a pin that carries the
    flag + deploy-history row.
16. **`goat-client-assay.yml` Phase 4 died on template recursion.** The play
    declared `assay_setup_key_ttl: "{{ assay_setup_key_ttl | default('48h') }}"`
    — a self-referencing play var. Fixed: defaults moved inline to the point
    of use.
17. **Enrollment 403'd at the Traefik mesh-only allowlist.** See §2 — the
    `mesh_only_allowlist_extra_cidrs` var now exists for exactly this, and
    passing the NATGW /32 is part of the canonical provision command.
18. **Peer cloud-init failed its package phase** (boot raced the NATGW), so
    peers had no AWS CLI and no installer — and cloud-init never re-runs.
    The §4 enroll command now self-heals both.
19. **`goat-clientd` cannot re-import a bundle over an existing profile.**
    `importBundle` fails with `profile: already exists: slug "default"` —
    so the installer's "idempotent, re-run freely" claim breaks the moment a
    first import persisted a profile (here: a bundle minted with a corrupted
    setup key). Workaround: stop the daemon, wipe `/var/lib/goat-client/`,
    re-run the installer. Field implication for bundle refresh on enrolled
    devices; needs a goat-client-side fix (replace-on-import or a
    `removeProfile` pre-step).
20. **The structural relay-forcing claim was falsified** — see the
    correction in §0. The host-level iptables block is required, and check E
    now fails loudly on any `direct` bridge.
21. **The headless embed is a netstack TUN** — no `wt0`, `wg(8)` sees
    nothing, `getStatus` has no local overlay IP, and OS-level ping over the
    overlay cannot work. run-assay.sh probes were rewritten against the IPC
    surface (`getStatus` / `getPeerConnectivity`) with python3 only, since
    the cloud-init failure (18) also meant no socat on the peers.

---

## 7. Teardown (MANDATORY)

Do this the same day. The simple path, preferred unless you have added other
resources to this tree in the meantime — remove or flip the line in
`terraform/terraform.auto.tfvars`:

```
goat_client_assay_enabled = false
```

then

```
aws-vault exec legacy --no-session -- terraform -chdir=terraform apply
```

Every resource in the file is count-gated on that variable, so this plans the
same destroy with no `-target` list to get wrong. If you must tear this env down
without touching anything else, use the explicit `-target` destroy list in the
header of `terraform/goat-client-assay.tf`.

Then the cleanup Terraform does not cover:

```
# 1. Staged artifacts — the bundles carry a device private key. The bucket's
#    1-day lifecycle would eventually take them, but do not wait for it.
aws-vault exec legacy --no-session -- \
  aws s3 rm "s3://$SSM_BUCKET/goat-client-assay/" --recursive

# 2. Local secrets + minted material.
rm -f /tmp/goat-assay-peer-*.cbor /tmp/assay-trust-roots.pem /tmp/assay-relays.json
rm -rf ~/.goat/state/assay-ca

# 3. The generated inventory (contains instance IDs that no longer exist).
rm -f ops/ansible/inventory/goat-client-assay.ini
```

Then confirm nothing survived:

```
aws-vault exec legacy --no-session -- aws ec2 describe-instances \
  --region <region> \
  --filters "Name=tag:Environment,Values=goat-client-assay" \
            "Name=instance-state-name,Values=running,stopped,pending" \
  --query 'Reservations[].Instances[].[InstanceId,Tags[?Key==`Name`].Value|[0]]' --output text

aws-vault exec legacy --no-session -- aws ec2 describe-nat-gateways \
  --region <region> \
  --filter "Name=tag:Environment,Values=goat-client-assay" \
  --query 'NatGateways[?State!=`deleted`].[NatGatewayId,State]' --output text

dig +short assay.legacy.example
dig +short relay-assay.legacy.example
```

All four should come back empty. A surviving NAT Gateway is the expensive
mistake here — it bills whether or not anything is behind it.

Finally, note the run in
cloud-env-lifecycle.md as RETIRED with the date and a
pointer to whatever evidence or finding came out of it. The setup key expires on
its own TTL, but the account row and the relay secret are gone with the host.
