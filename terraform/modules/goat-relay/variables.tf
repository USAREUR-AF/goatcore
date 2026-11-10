variable "relay_name" {
  description = "Short name for this relay (e.g., 'fra', 'isr', 'mum'). Empty = primary (no suffix)."
  type        = string
  default     = ""
}

variable "region" {
  description = "AWS region (used for subnet AZ)"
  type        = string
}

variable "project" {
  description = "Project name prefix"
  type        = string
}

variable "environment" {
  description = "Environment name"
  type        = string
}

variable "service_name" {
  description = "Short service name (e.g., 'goat-goat-relay')"
  type        = string
  default     = "goat-goat-relay"
}

variable "instance_type" {
  description = <<-EOT
    EC2 instance type. Default c7i.2xlarge per ADR 0712 (the architectural
    floor for the 8-shard goat-relay design — t3.micro's 2 vCPU caps shard
    count at 2 of 8 and wastes the lever stack). Documented fallback
    c6i.2xlarge if c7i unavailable in a region. NIC bump to c5n.2xlarge
    reserved for post-M-015 multi-pair characterization.
  EOT
  type        = string
  default     = "c7i.2xlarge"
}

variable "vpc_id" {
  description = "Existing VPC ID. Empty = create standalone VPC."
  type        = string
  default     = ""
}

variable "public_subnet_id" {
  description = "Existing public subnet ID. Required when vpc_id is set."
  type        = string
  default     = ""
}

variable "vpc_cidr" {
  description = "CIDR for auto-created VPC"
  type        = string
  default     = "10.99.1.0/24"
}

variable "ssh_cidr_blocks" {
  description = "CIDR blocks allowed for SSH. Default empty = SSM-only."
  type        = list(string)
  default     = []
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for DNS records"
  type        = string
}

variable "cloudflare_zone_name" {
  description = <<-EOT
    Cloudflare zone name (e.g. legacy.example, goat.example). Used by the
    relay_url output to produce the full FQDN. Default keeps legacy
    behavior; prod overrides to goat.example when this module is consumed
    from terraform-prod/.
  EOT
  type        = string
  default     = "legacy.example"
}

variable "relay_dns_prefix" {
  description = <<-EOT
    Prefix for per-relay public DNS records. Built as `<prefix>` (when
    relay_name is empty) or `<prefix>-<relay_name>`. Default `goat-relay`
    keeps the new tier separate from the legacy `relay-<name>` records
    so blue-green operates cleanly side-by-side.
  EOT
  type        = string
  default     = "goat-relay"
}

variable "round_robin_dns" {
  description = <<-EOT
    Add this relay to the goat-relay round-robin pool. Default true so
    that all three goat-relay-{fra,isr,mum} instances answer the same
    `<round_robin_dns_name>.<zone>` hostname. The legacy `netbird` pool
    is untouched — both pools coexist during blue-green cutover.
  EOT
  type        = bool
  default     = true
}

variable "round_robin_dns_name" {
  description = <<-EOT
    Hostname (sans zone) for the goat-relay round-robin record. Default
    `goat-relay` matches the legacy dev/test convention. Override
    per-env to avoid cross-tree collisions when this module is consumed
    from terraform-prod/ on the goat.example zone.
  EOT
  type        = string
  default     = "goat-relay"
}

variable "wg_cp0_enabled" {
  description = <<-EOT
    Open UDP port (default 51821) for the wg-cp0 silent control-plane
    WireGuard listener (Block 22 / ADR 0313). When true, the goat-relay
    box is also expected to run a wg-cp0 server (provisioned by the
    Ansible role wg-cp0-server). Mirrors the same flag on the legacy
    relay module — set true on the prod tree to give the new Block 33
    Rust tier active-active wg-cp0 forwarding parity with the legacy
    t3.micro tier (closes F-060's single-point-of-forwarding for
    cross-site Spock + mgmt-API + signal-fed traffic).
  EOT
  type        = bool
  default     = false
}

variable "wg_cp0_port" {
  description = "UDP port for the wg-cp0 listener (matches the legacy module's default)."
  type        = number
  default     = 51821
}

variable "rathole_ingress_enabled" {
  description = <<-EOT
    Open the colocated rathole ingress ports (TCP 443 + TCP 2333) on this
    goat-relay box. For goatnets whose mgmt-host is behind CGNAT (no public
    port-forward, e.g. ZAGROS on Starlink), the rathole-server runs ON the
    relay hosts so a brand-new off-LAN peer can reach the Netbird mgmt-API /
    signal at :443 before it has a mesh identity. Keeps the relay tier
    all-Rust (goat-relay for data) while colocating enrollment ingress — no
    separate legacy relay box. Deploy the server side with
    ops/ansible/relay-server-rathole.yml against [goat_relay_hosts].
  EOT
  type        = bool
  default     = false
}

variable "rathole_cidr_blocks" {
  description = "CIDRs allowed to reach the rathole control channel (TCP 2333). Public 443 stays open to 0.0.0.0/0; 2333 can be narrowed to the mgmt-host egress if known."
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "coturn_enabled" {
  description = <<-EOT
    Open coturn STUN ingress (UDP + TCP 3478) on this goat-relay box. The
    all-Rust relay tier dropped coturn (it lived only in the legacy Go
    relay-server.yml), so goatnets on it ran zero STUN — every peer
    relay-forced, no direct P2P. Set true alongside the Ansible
    coturn role (group_vars deploy_coturn=true) to restore STUN. STUN-only:
    this opens 3478 but NOT the TURN relay range — see coturn_turn_enabled.
  EOT
  type        = bool
  default     = false
}

variable "coturn_turn_enabled" {
  description = <<-EOT
    Additionally open the coturn TURN relay media range (UDP 49152-65535).
    TURN is deferred by default: the netbird rels:// relay already
    covers relay-fallback, so TURN's marginal value is low. Only meaningful
    when coturn_enabled is also true. Pair with Ansible coturn_turn_enabled +
    group_vars turn_hosts so management.json advertises the TURN servers.
  EOT
  type        = bool
  default     = false
}

variable "ab_stock_relay_port" {
  description = <<-EOT
    TEMPORARY A/B EXPERIMENT PORT. When > 0, opens TCP *and* UDP on this port
    in the relay security group so a STOCK upstream NetBird Go relay can run
    beside the production Rust goat-relay on the same host and carry the same
    traffic for comparison.

    Why a second port and not a second host: a relay is addressed as
    rels://host:port. Same host, same public IP, same DNS name and the same
    Let's Encrypt leaf (certs bind to the hostname, not the port) means the
    only variable left between the two arms is the relay implementation —
    which is the entire point of the experiment.

    Both protocols open together, per F-021: the Go relay serves
    WebSocket-over-TLS on TCP and QUIC on UDP at one address, so a TCP-only
    rule leaves the QUIC arm unreachable and the comparison silently becomes
    WS-vs-WS.

    Default 0 = no rule and no plan diff. Nothing is exposed until an operator
    sets it, and setting it back to 0 closes the surface again. The deploy half
    is ops/ansible/stock-relay-ab.yml; the cutover + rollback procedure is
    docs/operations/relay-ab-stock-vs-rust.md. Opening the port alone moves no
    traffic: peers only use relay endpoints management.json advertises.
  EOT
  type        = number
  default     = 0
}

variable "descriptions" {
  description = <<-EOT
    Resource description overrides, by key: security_group, ssh, websocket,
    quic, wg_cp0, rathole_ingress, rathole_control, coturn_stun_udp,
    coturn_stun_tcp, coturn_turn, ab_websocket, ab_quic, icmp, egress,
    ssh_key_secret, rathole_secret. A key not given keeps the module default
    (see locals.descriptions in main.tf). Pass a live goatnet's existing text
    so a plan shows no change: a security-group description cannot be edited
    in place, so a different one replaces the group.
  EOT
  type        = map(string)
  default     = {}

  validation {
    condition = length(setsubtract(keys(var.descriptions), [
      "security_group", "ssh", "websocket", "quic", "wg_cp0", "rathole_ingress",
      "rathole_control", "coturn_stun_udp", "coturn_stun_tcp", "coturn_turn",
      "ab_websocket", "ab_quic", "icmp", "egress", "ssh_key_secret", "rathole_secret",
    ])) == 0
    error_message = "descriptions has an unknown key; the keys are listed in this variable's description."
  }
}
