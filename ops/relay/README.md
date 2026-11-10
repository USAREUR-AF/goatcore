# VPS Relay

Each VPS relay runs up to three services: **rathole** forwards TCP 443 (gRPC management) to the network host behind the satellite link NAT, **Coturn** provides STUN on UDP 3478 for NAT traversal, and optionally **Netbird relay** (TCP 33073) handles data relay when P2P fails — eliminating the double-hop through the network host. DNS round-robin across all relays provides DDoS resilience and geographic distribution.

## Relays

Each relay is named by a short region code (for example `rla`, `rlb`, `rlc`)
and declared in the instance's Terraform tree and inventory. Every active relay
also carries an A record on `netbird.<zone>` (DNS round-robin). Clients resolve
any of them and connect to whichever relay answers.

## Architecture

```
                        +-----------------+
                        | netbird.        |
Remote Peer --DNS-----> | legacy.example |  (round-robin A records)
                        +--------+--------+
                    +------------+------------+
                    v            v            v
              +----------+ +----------+ +----------+
              | RLA VPS  | | RLB VPS  | | RLC VPS  |
              | rathole  | | rathole  | | rathole  |  TCP 443 → network host (forwarded)
              | Coturn   | | Coturn   | | Coturn   |  UDP 3478 STUN (local)
              | NB relay | | NB relay | | NB relay |  TCP 33073 data relay (local)
              | dnsmasq  | | dnsmasq  | | dnsmasq  |  UDP 53 mesh DNS (local)
              +----+-----+ +----+-----+ +----+-----+
                   |            |            |
                   +------------+------------+
                                |  rathole forwards TCP 443 only
                                v
                   +-------------------------+
                   | Network Host (behind the satellite link)|
                   | rathole-client@rla       |
                   | rathole-client@rlb       |
                   | rathole-client@rlc       |
                   |    |                     |
                   | localhost:443 (Traefik)  |
                   +-------------------------+
```

Each network host rathole client instance maintains a persistent outbound TCP 2333 control channel to its VPS. The VPS never initiates connections — it accepts the control channel and forwards incoming management traffic through it (NAT-friendly). STUN (UDP 3478) and data relay (TCP 33073) are handled locally at each VPS — they do not traverse rathole.

### Data flow by scenario

| Scenario | Path | Ports | Traverses rathole? |
|---|---|---|---|
| **Enrollment** | Peer → VPS rathole → network host management | TCP 443 | Yes |
| **Signal** | Peer → VPS rathole → network host signal | TCP 443 | Yes |
| **NAT traversal** | Peer → nearest VPS Coturn | UDP 3478 | No (local) |
| **Data relay** | Peer A → nearest VPS NB relay → Peer B | TCP 33073 | No (local) |
| **P2P** | Peer A ↔ Peer B (WireGuard direct) | UDP 51820 | No (direct) |
| **DNS** | Peer → network host or VPS dnsmasq | UDP 53 | No (mesh or direct) |

## Deploy a New Relay

All relays are managed in a single Terraform state (unified, no workspaces). Each region is a `module.relay_<name>` block in `terraform/main.tf`.

### Automated (recommended)

```bash
ops/relay/deploy-relay.sh          # Deploy/verify all regions
ops/relay/deploy-relay.sh rla      # Deploy/verify specific region
```

This orchestrates: Terraform → inventory generation → shared secrets → Ansible (VPS bootstrap + network host client config) → verification. Uses `ops/ansible/site.yml` which runs `relay-server.yml` (VPS) then `nethost-relay.yml` (network host).

See `ops/ansible/README.md` for Ansible-specific usage (targeting, extra-vars, Makefile).

### Manual steps

#### 1. Add module to Terraform

Add a new `module "relay_<name>"` block in `terraform/main.tf` with the appropriate provider alias. Add a provider alias for the new region if needed. Add the relay to `terraform/outputs.tf`.

```bash
cd terraform
aws-vault exec legacy --no-session -- terraform apply -input=false
```

#### 2. Bootstrap VPS (rathole + Coturn)

Via SSM (no SSH key needed):

```bash
aws-vault exec legacy --no-session -- \
  aws ssm send-command --region <aws-region> \
  --instance-ids <instance-id> \
  --document-name "AWS-RunShellScript" \
  --parameters '{"commands":["bash -s"]}'
```

Or via SSH:
```bash
# Get SSH key
aws-vault exec legacy --no-session -- \
  aws secretsmanager get-secret-value --region <aws-region> \
  --secret-id "<ssh-secret-name>" \
  --query SecretString --output text > /tmp/relay-key.pem
chmod 600 /tmp/relay-key.pem

ssh -i /tmp/relay-key.pem ubuntu@<VPS_IP> 'bash -s' < ops/relay/setup-vps.sh
rm /tmp/relay-key.pem
```

#### 3. Configure network host client

```bash
ssh -t goat@100.127.1.30 \
  RELAY_NAME=<name> \
  VPS_RELAY_IP=<ip> \
  VPS_RELAY_TOKEN=<token> \
  VPS_RELAY_PUBKEY=<pubkey> \
  'bash -s' < ops/relay/setup-client.sh
```

#### 4. Update STUN config

Add the new VPS IP to the `stuns` list in Netbird's `config.yaml` on the network host:

```yaml
server:
  stuns:
    - uri: "stun:<new-vps-ip>:3478"
```

Then restart: `cd /opt/netbird && sudo docker compose restart netbird-server`

#### 5. Store credentials

```bash
aws-vault exec legacy --no-session -- \
  aws secretsmanager put-secret-value --region <aws-region> \
  --secret-id "<rathole-secret-name>" \
  --secret-string '{"token":"...","public_key":"...","relay_ip":"...","turn_secret":"..."}'
```

#### 6. Verify

```bash
curl -sk --resolve netbird.legacy.example:443:<VPS_IP> https://netbird.legacy.example/api/accounts
# Expected: {"message":"no valid authentication provided","code":401}
```

## DDoS Resilience

| Layer | Protection | Cost |
|---|---|---|
| AWS Shield Standard | L3/L4 volumetric attack mitigation | Free (automatic on all EC2) |
| AWS Security Groups | Only required ports open (Terraform-managed) | Free |
| DNS round-robin | If one VPS is DDoS'd, remove its A record | Free (Terraform: set `management_dns = false`) |
| Geographic distribution | Attacker must hit region 1 AND region 2 AND region 3 | Architectural |

To remove a relay from the DNS pool during an attack:

```bash
# In terraform/main.tf, set management_dns = false on the affected module
# Then:
cd terraform
aws-vault exec legacy --no-session -- terraform apply -input=false
```

## Files

| File | Purpose |
|---|---|
| `deploy-relay.sh` | Top-level orchestrator (Terraform → Ansible → verify) |
| `setup-vps.sh` | Standalone VPS bootstrap script (bash fallback for Ansible) |
| `setup-client.sh` | Standalone network host rathole client script (bash fallback for Ansible) |
| `update-stun-config.sh` | Standalone STUN/relay config update (bash fallback for Ansible) |
| `../ansible/relay-server.yml` | Ansible: VPS relay bootstrap (rathole + Coturn + NB relay) |
| `../ansible/nethost-relay.yml` | Ansible: network host client config (rathole clients + STUN/relay) |
| `../ansible/site.yml` | Ansible: runs both playbooks in order |

### Network host multi-relay layout

```
/etc/rathole/
  client-rla.toml    -> rathole-client@rla.service
  client-rlb.toml    -> rathole-client@rlb.service
  client-rlc.toml    -> rathole-client@rlc.service
```

## Relay TLS

The Netbird relay on each VPS terminates TLS for peer connections on port 33073. TLS certs are obtained by **certbot** on the VPS host (standalone HTTP-01 on port 80), NOT by the relay binary's built-in Let's Encrypt client.

**Why not the relay binary's built-in Let's Encrypt:**

- The binary's `--letsencrypt-domains` uses TLS-ALPN-01 on port 443, but rathole occupies port 443
- It requires `--letsencrypt-data-dir` which silently does nothing if omitted
- No error, no cert, no log -- the container appears healthy but TLS handshakes fail

**How it works:**

1. Ansible installs certbot and runs `certbot certonly --standalone -d <relay-dns-name>`
2. The full `/etc/letsencrypt` directory is mounted into the container (`-v /etc/letsencrypt:/etc/letsencrypt:ro`). Mounting only `live/` does not work because cert files are symlinks to `../../archive/`.
3. The relay binary starts with `--tls-cert-file` and `--tls-key-file` pointing to the mounted cert paths
4. Certbot auto-renews via systemd timer. The relay container must be restarted to pick up renewed certs.

**DNS names:** region 1 is `relay.legacy.example` (no region suffix). region 2 and region 3 are `relay-rlb.legacy.example` and `relay-rlc.legacy.example`.

## Security

- **Noise Protocol (Noise_NK_25519_ChaChaPoly_BLAKE2s):** Control channel between VPS and the network host is encrypted and authenticated. Server identity verified by public key pinned in client config.
- **Auth token:** Shared secret authorizing which services the client can forward.
- **AWS Security Groups:** VPS allows only required ports. Managed by Terraform (`modules/relay/main.tf`).
- **Credentials in Secrets Manager:** Rathole tokens and public keys stored per-region. SSH keys stored separately.

## Monitoring

```bash
# VPS: check all three services
sudo systemctl status rathole-server coturn
sudo docker ps --filter name=netbird-relay
sudo journalctl -u rathole-server -f
sudo journalctl -u coturn -f
sudo docker logs -f netbird-relay

# Network host: check all relay clients
sudo systemctl status 'rathole-client@*'
sudo journalctl -u 'rathole-client@rla' -f
```

## Troubleshooting

| Problem | Check |
|---|---|
| `curl` to VPS:443 times out | AWS SG allows TCP 443? rathole-server running? |
| rathole client won't connect | VPS:2333 reachable from the network host? `nc -z -w 5 VPS_IP 2333` |
| Client connects but no forwarding | Token mismatch? Service names must match |
| STUN not working | Coturn running? AWS SG allows UDP 3478? |
| Client disconnects frequently | Increase `heartbeat_timeout` if RTT is high |
| Netbird relay not reachable | Docker running? `docker ps -a` (container may have been removed after crash-loop). AWS SG allows TCP 33073? |
| Relay container won't start | Check `NB_RELAY_AUTH_SECRET` was passed. Check TLS cert exists: `ls /etc/letsencrypt/live/`. `docker logs netbird-relay` |
| Relay TLS cert missing | certbot failed or was never run. Run: `certbot certonly --standalone -d <relay-dns-name>`. Port 80 must be free. |
| Relay container crash-loops and disappears | Missing TLS cert file. The relay binary exits immediately if cert path does not exist. Check `docker ps -a` and `docker logs`. |
| Peers show `rels://netbird.legacy.example:443` | VPS relay TLS is broken. Check relay container logs and cert. Peers silently fall back to management server relay. |
| DNS queries to relay time out | AWS SG missing UDP 53? `dig +short +time=3 @<relay-ip> airdefenders.netbird.legacy.example` |
| dnsmasq running but not answering | Check listen address: must be EC2 private IP, not `0.0.0.0`. `ss -ulnp \| grep :53` |
| Peer DNS works but service DNS doesn't | Nameserver group DNS (dnsmasq) is separate from Netbird peer DNS. See `docs/design/dns-service-discovery.md` |

## Cost

| Item | Per relay | Total (3 relays) |
|---|---|---|
| EC2 t3.micro | ~$8/mo | ~$24/mo |
| Elastic IP | Free (attached) | Free |
| Data transfer | Low (management only) | Free tier likely |
