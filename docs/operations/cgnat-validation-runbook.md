# CGNAT-aware signal federation validation — operator runbook

End-to-end operator runbook for the ephemeral CGNAT validation env.
Design: [`../design/cgnat-validation-env.md`](../design/cgnat-validation-env.md);
why attempt-1 was retired for Path C is its §2.1.

The Terraform root, the inventory and any script that writes it belong to
the deployment and live in its overlay repository; none of them is in this
repository. The orchestrator is
[`../../ops/ansible/cgnat-validation.yml`](../../ops/ansible/cgnat-validation.yml).

**Currently targeting:** attempt-2, Path C — 100 × t2.nano native-netbird
peers (50 per site). Attempt-1's nested-container swarm is retired.

**Estimated wall-clock:** ~30 h total (provision 15 min + enroll 15 min +
24 h soak + faults 1 h + evidence 30 min + teardown 5 min). Cost ~$16.

**Prerequisites:**

- `aws-vault` profile `legacy` active.
- `terraform` ≥ 1.6, `ansible` ≥ 2.16, `community.aws` collection.
- One 32-byte pg superuser password (the playbook uses it on both sites) + one 32-byte netbird DataStoreEncryptionKey, exported so the play can read them from its environment. **The netbird key MUST be generated with `openssl rand -base64 32`, not `-hex 32`** — netbird-mgmt's boot path base64-decodes the value and asserts decoded length == 32; hex-32 produces 64 chars which decode to 48 bytes and FATL the mgmt-plane.

  ```bash
  export CGNAT_PG_PASS="$(openssl rand -hex 32)"      # any 32+ bytes; pgx pgpass-safe (no /, +, =)
  export CGNAT_NB_ENCKEY="$(openssl rand -base64 32)" # 44 chars including padding; decodes to 32 bytes
  export CGNAT_SITE_A_SETUP_KEY="$(uuidgen)" CGNAT_SITE_B_SETUP_KEY="$(uuidgen)"
  ```

  Give Terraform the same two setup keys: the peers' cloud-init user_data
  carries them.

---

## 1. Provision

In the deployment's Terraform root for this env (in its overlay repository):

```bash
aws-vault exec legacy --no-session -- terraform apply \
  -target=aws_iam_role.cgnat_ssm \
  -target=aws_iam_role_policy_attachment.cgnat_ssm \
  -target=aws_iam_instance_profile.cgnat_ssm \
  -target=aws_subnet.cgnat_priv_a \
  -target=aws_subnet.cgnat_priv_b \
  -target=aws_eip.cgnat_natgw_a \
  -target=aws_eip.cgnat_natgw_b \
  -target=aws_nat_gateway.cgnat_a \
  -target=aws_nat_gateway.cgnat_b \
  -target=aws_route_table.cgnat_priv_a \
  -target=aws_route_table.cgnat_priv_b \
  -target=aws_route_table_association.cgnat_priv_a \
  -target=aws_route_table_association.cgnat_priv_b \
  -target=aws_security_group.cgnat_mgmt_a \
  -target=aws_security_group.cgnat_mgmt_b \
  -target=aws_security_group.cgnat_peer_a \
  -target=aws_security_group.cgnat_peer_b \
  -target=aws_instance.cgnat_mgmt_a \
  -target=aws_instance.cgnat_mgmt_b \
  -target=aws_eip.cgnat_mgmt_a \
  -target=aws_eip.cgnat_mgmt_b \
  -target=aws_instance.cgnat_peer_host_a \
  -target=aws_instance.cgnat_peer_host_b

# Capture outputs
terraform output cgnat_mgmt_a_instance_id
terraform output cgnat_mgmt_a_public_ip
terraform output cgnat_mgmt_b_instance_id
terraform output cgnat_mgmt_b_public_ip
terraform output cgnat_peer_host_a_instance_ids
terraform output cgnat_peer_host_b_instance_ids
```

Write the inventory from those outputs, in the deployment's overlay: two
groups, `cgnat_mgmt_a_hosts` and `cgnat_mgmt_b_hosts`, one mgmt host each,
with `signal_uri` and `site_name` set per host. Every phase of the
playbook targets those two groups.

Wait ~90 s for cloud-init to complete on all 102 hosts (2 mgmt + 100
peer). SSM agent comes up as soon as the base Ubuntu boot finishes; the
peer-node cloud-init only installs curl/gnupg so it's fast.

## 2. Deploy mgmt-plane + enroll 100 native peers

Each secret is read by the play from its own environment
(`lookup("env", ...)` inside single quotes), so the shell puts no value on
the command line:

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  aws-vault exec legacy --no-session -- \
  ansible-playbook \
    -i <the deployment's inventory> \
    cgnat-validation.yml \
    -e 'postgres_superuser_password={{ lookup("env", "CGNAT_PG_PASS") }}' \
    -e 'netbird_encryption_key={{ lookup("env", "CGNAT_NB_ENCKEY") }}' \
    -e 'cgnat_site_a_setup_key={{ lookup("env", "CGNAT_SITE_A_SETUP_KEY") }}' \
    -e 'cgnat_site_b_setup_key={{ lookup("env", "CGNAT_SITE_B_SETUP_KEY") }}'
```

Phases:

1. **Phase 1** — mgmt-host.yml against both sites (postgres-spock +
   signal-server + netbird-management).
2. **Phase 2** — socat sidecar exposing mgmt API on `0.0.0.0:80 →
   127.0.0.1:8080` (the base compose binds loopback-only; this is the
   validated workaround from attempt-1 findings §2.4).
3. **Phase 3** — seed `accounts` + `setup_keys` rows in each site's
   mgmt DB via `docker exec postgres-a psql`.
4. **Phase 4** — removed from the playbook: each peer enrolls itself at
   first boot from its cloud-init user_data (`netbird up --setup-key …`,
   retried), so Ansible does not fan out across the peers.
5. **Phase 5** — smoke: 3 min pause, query signal /metrics + peer
   counts.

Phase 5 should report site-α ≥ 45 peers and site-β ≥ 45 peers. If under
40, inspect the peers' cloud-init logs over SSM — typical failures are
transient pkgs.netbird.io fetch errors, which the user_data retries.

## 3. 24 h soak (hours 0–20)

Let it run. Periodically sample via SSM:

```bash
# Signal metrics — watch connected_peers + forward counters grow
aws-vault exec legacy --no-session -- aws ssm send-command \
  --region <cloud-region-1-notional> --instance-ids <mgmt-a-id> \
  --document-name AWS-RunShellScript \
  --parameters 'commands=["curl -sS http://127.0.0.1:9090/metrics | grep -E \"signal_server_(connected_peers|forward_sent_total|forward_received_total)\""]'
```

## 4. Fault injection (hours 20–21)

Run three fault scenarios against this env while the load runs: (1) kill
signal-a and measure the impact, (2) add 2000 ms of latency on signal-a's
egress with `tc netem`, and (3) verify recovery. The load generator is
[`../../signal-server/cmd/federation-load/main.go`](../../signal-server/cmd/federation-load/main.go);
the scenario scripts are not in this repository.

## 5. Evidence capture (hour 21–22)

Capture per-site signal metrics + peer status + data-plane sampling:

```bash
for site in a b; do
  aws-vault exec legacy --no-session -- aws ssm send-command \
    --region $([ "$site" = "a" ] && echo <cloud-region-1-notional> || echo <cloud-region-alt-notional>) \
    --instance-ids <mgmt-$site-id> \
    --document-name AWS-RunShellScript \
    --parameters 'commands=["curl -sS http://127.0.0.1:9090/metrics > /tmp/signal-metrics-'$site'.txt; sudo docker exec postgres-a psql -U postgres netbird -c \"SELECT COUNT(*), connection_status FROM peers GROUP BY connection_status;\" > /tmp/peer-status-'$site'.txt"]'
done
```

psql connects over the container's local socket, as the playbook's own
reloads do, so the command carries no password.

File the evidence with the deployment's records, in its overlay
repository.

## 6. Pass / fail verdict

Per design doc §3 pass criteria:

- `signal_server_connected_peers` ≥ 45 on each side, held continuously.
- `forward_sent_total{outcome="delivered"}` > 100/hr per site.
- `forward_sent_total{outcome="deadline_exceeded"}` < 0.1 % of delivered.
- Relay fallback rate ≥ 80 % of cross-site peer-pair establishments.
- No unbounded latency / error growth in scenarios 1–3.

## 7. Teardown (MANDATORY)

**Run immediately after pass/fail verdict is captured. Do NOT leave the
env running.**

Teardown: `terraform destroy` in the deployment's Terraform root for this
env, with the same `-target` list as §1.

Also:

- Discard this run's inventory (it names instance IDs that no longer exist).
- Record the env as retired in the deployment's records.
- Revoke the 1Password items for this run's secrets.
