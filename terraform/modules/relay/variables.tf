variable "relay_name" {
  description = "Short name for this relay (e.g., 'isr', 'mum'). Empty = primary (no suffix)."
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
  description = "Short service name"
  type        = string
}

variable "instance_type" {
  description = "EC2 instance type"
  type        = string
  default     = "t3.micro"
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
  default     = "10.99.0.0/24"
}

variable "ssh_cidr_blocks" {
  description = "CIDR blocks allowed for SSH"
  type        = list(string)
  default     = []
}

variable "http_port_open" {
  description = <<-EOT
    Open TCP/80 for ACME HTTP-01 challenge. Defaults to true for
    backwards compatibility with the legacy + prod relay tiers; set
    to false when the Traefik chain uses DNS-01 exclusively (nova)
    to drop the scannable surface. See
    docs/design/nova-public-posture.md.
  EOT
  type        = bool
  default     = true
}

variable "rathole_cidr_blocks" {
  description = "CIDR blocks allowed to reach the rathole control channel (TCP 2333). Restrict to NETHOST egress IP when known."
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for DNS records"
  type        = string
}

variable "cloudflare_zone_name" {
  description = <<-EOT
    Cloudflare zone name (e.g. legacy.example, goat.example). Used by the
    relay_url output to produce the full FQDN. Default keeps legacy
    behavior; prod overrides to goat.example.
  EOT
  type        = string
  default     = "legacy.example"
}

variable "management_dns" {
  description = "Add this relay to the management DNS round-robin pool"
  type        = bool
  default     = true
}

variable "management_dns_name" {
  description = <<-EOT
    Hostname (sans zone) for the management DNS round-robin record. Default
    matches the legacy dev/test convention (`netbird` → `netbird.legacy.example`).
    Override per-env to avoid cross-tree collisions, e.g. `netbird-prod`
    when this module is consumed from terraform-prod/ on the goat.example zone.
  EOT
  type        = string
  default     = "netbird"
}

variable "relay_dns_prefix" {
  description = "Prefix for per-relay public DNS records. The module builds `<prefix>` (when relay_name is empty) or `<prefix>-<relay_name>` (otherwise). Default `relay` keeps existing legacy behavior. Override per-env when needed."
  type        = string
  default     = "relay"
}

variable "status_dns" {
  description = "Add this relay to the status DNS round-robin pool"
  type        = bool
  default     = false
}

variable "status_dns_name" {
  description = <<-EOT
    Hostname (sans zone) for the status DNS round-robin record. Default
    `isitup.netbird` matches the legacy convention. Override per-env
    when needed.
  EOT
  type        = string
  default     = "isitup.netbird"
}

variable "wg_cp0_enabled" {
  description = <<-EOT
    Open UDP port (default 51821) for the wg-cp0 silent control-plane
    WireGuard listener (Block 22 / ADR 0313). When true, the relay is
    expected to run a wg-cp0 server (provisioned by the Ansible role
    wg-cp0-server). legacy tree leaves this false (legacy rathole
    pattern); terraform-prod sets true.
  EOT
  type        = bool
  default     = false
}

variable "wg_cp0_port" {
  description = "UDP port for the wg-cp0 listener. Configurable per-relay if defense-in-depth against port-scanning fingerprinting matters."
  type        = number
  default     = 51821
}

variable "quic_rule_description" {
  description = "Description of the UDP QUIC ingress rule. A goatnet with live relays passes the text its rule already carries: changing a rule description rewrites the security group's rules."
  type        = string
  default     = "Netbird relay QUIC data relay"
}

variable "icmp_rule_description" {
  description = "Description of the ICMP ingress rule. A goatnet with live relays passes the text its rule already carries."
  type        = string
  default     = "ICMP ping (Satlink PoP benchmarks)"
}

variable "purpose_tag" {
  description = "Value of the instance's Purpose tag."
  type        = string
  default     = "rathole VPS relay for Netbird management behind the satellite link NAT"
}
