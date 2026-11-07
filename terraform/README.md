# terraform/ — reusable cloud modules

goatcore ships Terraform modules only. The root that applies them belongs to
your goatnet's overlay repository, because a root names your account, your
regions and your DNS zone. The worked example is
[goatenv/terraform-zagros](https://github.com/usareur-af/goatenv/tree/main/terraform-zagros):
two relays in your own AWS account, DNS through Cloudflare, consumed from this
repository by tag:

```hcl
module "relay_a" {
  source = "git::https://github.com/usareur-af/goatcore.git//terraform/modules/goat-relay?ref=goatcore/v0.1.0"
  ...
}
```

A LAN-only goatnet, and every goatlite, needs none of this.

## Modules

| Module | What it creates | When you need it |
|---|---|---|
| `goat-relay` | One relay VPS: EC2 instance, Elastic IP, security group, SSM role, SSH key in Secrets Manager, optional VPC; runs the goat relay binary | Every goatnet reachable over the internet. Two in different regions is the usual shape |
| `relay` | The same shape running the stock NetBird relay | Only if you choose the stock relay over goat's |
| `bridge` | A VPS that joins the mesh and forwards named TCP ports to an outside endpoint over socat | Reaching a partner system that is not a mesh peer |
| `goat-public-mtls-crutch` | A public mTLS-gated reachability tier, one VPS per region, operator-toggleable | Clients that cannot form a WireGuard path; default off |
| `host-node` | One node of a four-node host site on EC2: core pair and observability pair, data volume, SSM-managed, enrolled into the mesh by cloud-init | A cloud-hosted site instead of hardware |
| `mesh-dns-host` | A small always-on host for mesh DNS, SSM-managed, port 22 closed | A goatnet whose discovery DNS should outlive any one site |
| `relay-health-host` | An always-on host for the relay-health surface, same posture as `mesh-dns-host` | Watching relays from outside the mesh |

Every module is SSM-managed: Ansible reaches the host with port 22 closed, so
the operator does not have to be on the mesh to converge it.

## How the root connects to the standup

`ops/ansible/inventory/generate-relays.sh` reads the root's `relays` output and
writes the relay inventory, so `goatbuilder standup` finds the relays with no
further wiring. See `docs/operations/goatnet-standup.md`.

## Resource descriptions are inputs

`modules/goat-relay` and `modules/relay-health-host` take a `descriptions` map.
A security-group description cannot be changed in place, so a root whose live
goatnet was created with other text passes that text and plans no change:

```hcl
module "relay_a" {
  source = "../terraform/modules/goat-relay"
  # ...
  descriptions = { icmp = "<the live rule's description>" }
}
```

Keys: `goat-relay` — `security_group`, `ssh`, `websocket`, `quic`, `wg_cp0`,
`rathole_ingress`, `rathole_control`, `coturn_stun_udp`, `coturn_stun_tcp`,
`coturn_turn`, `ab_websocket`, `ab_quic`, `icmp`, `egress`, `ssh_key_secret`,
`rathole_secret`; `relay-health-host` — `security_group`, `public_443`,
`public_80`. A key not given keeps the module default.
