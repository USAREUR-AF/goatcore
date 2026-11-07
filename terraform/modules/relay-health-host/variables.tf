# relay-health-host — an always-on cloud host for the cloud-resident
# relay-health surface (ADR 1061). Cloned from mesh-dns-host (ADR 1056).

variable "name" {
  description = "Short host name suffix, e.g. 'a' (host becomes relay-health-<goatnet>-<name>)."
  type        = string
  default     = "a"
}

# Which goatnet this host belongs to. Threads through every resource name, the
# SG description and the NetBird enrollment hostname, so a second goatnet's box
# does not collide with another's. DEFAULTS TO "zagros" ON PURPOSE: with this value
# every generated name is byte-identical to what the module produced before it
# was parameterized, so an existing host is untouched — no rename, no
# destroy/recreate. A new goatnet passes its own id.
variable "goatnet" {
  description = "Goatnet id this host serves (host becomes relay-health-<goatnet>-<name>). Default 'zagros' preserves the pre-parameterization names."
  type        = string
  default     = "zagros"
}

variable "az" {
  description = "Availability zone to place the instance in."
  type        = string
  default     = "eu-central-1a"
}

variable "instance_type" {
  description = "Instance type. The surface (Traefik + oauth2-proxy + a static dashboard + a tiny ingest + a prober) is light; a small burstable is plenty."
  type        = string
  default     = "t3.small"
}

variable "root_volume_size" {
  description = "Root EBS size (GiB)."
  type        = number
  default     = 20
}

variable "netbird_setup_key" {
  description = "Plaintext ZAGROS NetBird setup-key (TF_VAR_zagros_netbird_setup_key) for mesh enrollment. Must be valid + non-exhausted."
  type        = string
  sensitive   = true
}

variable "netbird_mgmt_url" {
  description = "NetBird management URL for enrollment. The default is ZAGROS's — a second goatnet MUST pass its own, or the host enrolls on the wrong mesh."
  type        = string
  default     = "https://netbird.zagros.example:443"
}

variable "ssm_bucket_arn" {
  description = "ARN of the ZAGROS SSM S3 bucket so the ansible aws_ssm connection can transfer files to this host."
  type        = string
}

# PHASE GATE. false (default) = PHASE 1: egress-only SG, no EIP, no public
# surface. true = PHASE 2: attach an Elastic IP + open :443/:80 — the single
# deliberate, operator-watched public flip (paired with the Cloudflare A-record
# + the Dex oauth2-proxy client).
variable "public_enabled" {
  description = "PHASE 2 gate: attach an Elastic IP + open :443/:80. Default false = PHASE 1 (no public exposure)."
  type        = bool
  default     = false
}

variable "public_ingress_cidrs" {
  description = "CIDRs allowed to reach :443/:80 when public_enabled. Default open to the internet (the break-glass surface); tighten if desired."
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "tags" {
  description = "Resource tags (merged with compliance default_tags by the root module)."
  type        = map(string)
  default     = {}
}

variable "descriptions" {
  description = <<-EOT
    Resource description overrides, by key: security_group, public_443,
    public_80. A key not given keeps the module default (locals.descriptions
    in main.tf). Pass a live goatnet's existing text so a plan shows no
    change: a security-group description cannot be edited in place, so a
    different one replaces the group.
  EOT
  type        = map(string)
  default     = {}

  validation {
    condition     = length(setsubtract(keys(var.descriptions), ["security_group", "public_443", "public_80"])) == 0
    error_message = "descriptions has an unknown key: use security_group, public_443 or public_80."
  }
}
