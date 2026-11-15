# Fresh Deployment Checklist

Everything a new team needs to deploy goat from scratch on new accounts and hardware.

## Prerequisites

### Accounts

| Account | Purpose | Setup |
|---------|---------|-------|
| AWS | VPS relays, bridges, SSM, Secrets Manager | Create account, configure `aws-vault add <profile>` |
| Cloudflare | DNS, TLS certificates, DNSSEC | Register domain, create API token (see below) |
| Anthropic (optional) | AI analysis in secwatch | An API key from the provider's console, exported as `ANTHROPIC_API_KEY` (below) |

### Cloudflare API token

Create one token named `<project>-infra` at [dash.cloudflare.com/profile/api-tokens](https://dash.cloudflare.com/profile/api-tokens):

| Scope | Resource | Permission |
|-------|----------|------------|
| Zone | DNS | Edit |
| Zone | Zone | Read |
| Zone | Zone Settings | Edit |
| Zone | SSL and Certificates | Edit |

Zone resources: Include → Specific zone → your domain.

This single token covers Terraform (DNS records, DNSSEC, Universal SSL), Traefik (Let's Encrypt DNS-01 cert renewal), and secwatch (DNS checks). Cloudflare may auto-create dashboard-internal tokens — leave those alone.

### Tools (local machine)

```
brew install ansible aws-vault direnv python3 terraform
brew install netbirdio/tap/netbird
pip3 install pyyaml
```

### Hardware

- x86 mini PC (or equivalent) running Ubuntu 24.04 LTS
- Ethernet connection to a switch with internet access
- No port forwarding required (VPS relays provide ingress)

## Environment Variables

### `.envrc` (committed, non-secret)

```bash
# Mesh
export NETHOST_HOST=<nethost-mesh-ip>          # assigned after mesh enrollment
export NETHOST_USER=goat
export NETBIRD_DOMAIN=netbird.<your-domain>
export CF_ZONE=<your-domain>
export CF_ACCOUNT_ID=<cloudflare-account-id>

# AWS
export AWS_DEFAULT_REGION=<cloud-region-1-notional>    # or your preferred region
export AWS_VAULT_KEYCHAIN_NAME=login
export AWS_SDK_LOAD_CONFIG=1

# macOS Ansible fix
export OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES

# Load secrets
source_env_if_exists .envrc.local

# Terraform Cloudflare provider
export CLOUDFLARE_API_TOKEN="${CLOUDFLARE_API_TOKEN:-$CF_API_TOKEN}"
```

### `.envrc.local` (gitignored, secrets)

Back up all values in 1Password or equivalent.

```bash
# Required for all deployments
export NETHOST_PASS=<ssh-password>
export CF_API_TOKEN=<cloudflare-api-token>       # See "Cloudflare API token" section for required permissions
export NETBIRD_SETUP_KEY=<netbird-setup-key>     # created in dashboard after Step 3
export NETBIRD_API_TOKEN=<netbird-pat>           # Personal Access Token from dashboard

# Cloudflare Zone ID (for secwatch DNS checks)
export CF_ZT_ZONE_ID=<cloudflare-zone-id>

# Optional
export ANTHROPIC_API_KEY=<key>                   # enables secwatch AI analysis
export TAK_HOST=<tak-mesh-ip>                    # if deploying TAK server
```

### AWS credentials

Managed by `aws-vault`, never in env files:

```bash
aws-vault add <profile-name>
# Enter access key ID and secret access key
```

Every `terraform` and `ansible-playbook` command runs through `aws-vault exec <profile> --no-session --`.

## Deployment Steps

### 1. Network host initial setup

Follow `docs/operations/deploy-guide.md` Steps 0-6:
- SSH key auth
- Docker install
- Netbird self-hosted deployment with DNS-01 TLS

### 2. Cloudflare zone configuration (one-time)

Terraform automates Universal SSL disable and DNSSEC signing. After the first `terraform apply`, one manual step remains — register the DS record at Route 53:

```bash
# Wait ~1-5 min after apply for DNSKEY to propagate, then:
aws-vault exec <profile> --no-session -- \
  aws route53domains associate-delegation-signer-to-domain \
  --region us-east-1 \
  --domain-name <your-domain> \
  --signing-attributes "Algorithm=13,Flags=257,PublicKey=$(dig DNSKEY <your-domain> +short | grep '257' | awk '{print $4}')"
```

Must use `--region us-east-1` — Route 53 Domains API only works in that region. Only needed once per zone creation.

### 3. Infrastructure (Terraform)

```bash
cd terraform
aws-vault exec <profile> --no-session -- terraform init
aws-vault exec <profile> --no-session -- terraform apply
```

Creates: VPS relays, bridge VPSes, DNS records, security groups, Secrets Manager entries, SSM S3 bucket.

### 4. Generate inventory

```bash
cd ops/ansible
inventory/generate.sh
```

Reads Terraform outputs, writes:
- `inventory/hosts.ini` (Ansible inventory with SSM connection details)
- `ops/relay-env.sh` (relay metadata for bash scripts)

Re-run after any Terraform changes (new relays, bridges, IP changes).

### 5. Generate service configs

```bash
python3 ops/serve/generate.py
```

Reads `ops/serve/services.yaml`, writes:
- `ops/serve/caddy/Caddyfile`
- `ops/serve/docker-compose.yml`
- `ops/status/services.env`
- `ops/dns/mesh-hosts` (service DNS entries)

Re-run after adding/removing services.

### 6. Deploy everything

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec <profile> --no-session -- \
  ansible-playbook -i inventory/hosts.ini site.yml
```

Or by tier:

```bash
# Tier 1: VPS relays + network host infra
make infra

# Tier 2: Network host services
make services

# Bridges (first run needs setup key)
ansible-playbook bridge.yml -e netbird_setup_key=$NETBIRD_SETUP_KEY
```

### 7. Verify

```bash
make verify                    # relay connectivity
curl -sk https://isitup.<domain>/  # status page
curl -sk https://secwatch.<domain>/api/results/latest  # security scan
```

### 8. Enroll peers

On each new device:

```bash
sudo netbird up --management-url https://netbird.<domain> --setup-key $NETBIRD_SETUP_KEY
```

## Adding a New Service

1. Add entry to `ops/serve/services.yaml`
2. If container type: create docker-compose.yml with required Traefik labels (see below)
3. If container type: create Ansible playbook in `ops/ansible/`
4. Run `python3 ops/serve/generate.py` (updates Caddyfile, services.env, mesh-hosts)
5. Add playbook import to `site.yml` and `site-services.yml`
6. Deploy: `make services`

### Required Traefik labels for container services

```yaml
labels:
  - "traefik.enable=true"
  - "traefik.docker.network=netbird_netbird"
  - "traefik.http.routers.<name>.rule=Host(`<name>.netbird.<domain>`)"
  - "traefik.http.routers.<name>.entrypoints=websecure"
  - "traefik.http.routers.<name>.tls=true"
  - "traefik.http.routers.<name>.middlewares=mesh-only"
  - "traefik.http.services.<name>.loadbalancer.server.port=<port>"
```

`generate.py` validates these labels and exits non-zero if any are missing.
Do NOT add `tls.certresolver=letsencrypt` — the wildcard-cert router on caddy-proxy
obtains a single cert covering all `*.netbird.legacy.example` hostnames.

## What Gets Auto-Generated vs Manual

| Artifact | Source | Generated by |
|----------|--------|--------------|
| Ansible inventory | Terraform outputs | `inventory/generate.sh` |
| Relay bash metadata | Terraform outputs | `inventory/generate.sh` |
| Caddyfile | `services.yaml` | `generate.py` |
| Caddy docker-compose | `services.yaml` | `generate.py` |
| Health check targets | `services.yaml` | `generate.py` |
| Mesh DNS entries | `services.yaml` | `generate.py` |
| Docker log rotation | Ansible | `nethost-docker.yml` / `relay-server.yml` |
| Cloudflare DNS records | Terraform | `terraform apply` |

## Troubleshooting

- **macOS DNS cache after adding service**: `sudo dscacheutil -flushcache && sudo killall -HUP mDNSResponder`
- **Ansible SSH timeout**: Transient; retry the command
- **Traefik returns 404**: Request used IP instead of hostname. Test with `-H "Host: <name>.netbird.<domain>"`
- **SSM connection fails**: Verify S3 bucket exists (`terraform output ansible_ssm_bucket`) and `aws-vault` credentials are fresh
- **VS Code terminal can't reach mesh**: Use a standalone terminal for SSH
