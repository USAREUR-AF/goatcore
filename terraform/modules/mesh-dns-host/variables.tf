# mesh-dns-host — a lightweight, always-on mesh node that runs ONLY a
# CoreDNS replica (ADR 1056 / docs/design/mesh-dns-redundancy.md). It joins the
# netbird mesh via cloud-init (setup key) so peers can query it by mesh IP,
# and is reached for configuration via SSM (port 22 closed, like the relays) —
# NOT over the mesh, so the operator does not need to be on the mesh to
# deploy CoreDNS to it. CoreDNS itself is deployed by ops/ansible/goat-coredns.yml.

variable "name" {
  description = "Short host name suffix, e.g. 'a' / 'b' (host becomes mesh-dns-<goatnet>-<name>)."
  type        = string
}

variable "goatnet" {
  description = "Goatnet id this host serves; it threads through every resource name (mesh-dns-<goatnet>-<name>). Changing it on a live host renames, and so replaces, its IAM role, instance profile and security group."
  type        = string
  default     = "zagros"
}

variable "label" {
  description = "Label used in the security-group description and the boot log lines. Defaults to the goatnet id upper-cased."
  type        = string
  default     = null
}

variable "az" {
  description = "Availability zone to place the instance in (path diversity across the 2 replicas)."
  type        = string
}

variable "instance_type" {
  description = "Instance type. CoreDNS is tiny; a small burstable is plenty."
  type        = string
  default     = "t3.small"
}

variable "root_volume_size" {
  description = "Root EBS size (GiB). Generous vs the 19 GB relays since this host is log-bounded and single-purpose."
  type        = number
  default     = 20
}

# NOTE (enrollment foot-gun): a NetBird setup-key expires + has a
# usage limit. If the setup-key variable is
# stale/exhausted, cloud-init's `netbird up` fails ("setup key is invalid") and
# the host enrolls on nothing — the bootstrap log shows the retries. Mint a
# fresh reusable key (POST /api/setup-keys; "All" is NOT a valid auto_group —
# use a real group) and either refresh the env var before apply, or re-run
# `netbird up --setup-key <fresh>` on the host over SSM (no instance churn).
variable "netbird_setup_key" {
  description = "Plaintext ZAGROS NetBird setup-key (TF_VAR_zagros_netbird_setup_key from ZAGROS_NETBIRD_SETUP_KEY) for mesh enrollment. Must be VALID + non-exhausted (see note above)."
  type        = string
  sensitive   = true
}

variable "netbird_mgmt_url" {
  description = "NetBird management URL for enrollment. A goatnet passes its own."
  type        = string
  default     = "https://netbird.zagros.example:443"
}

variable "ssm_bucket_arn" {
  description = "ARN of the ZAGROS SSM S3 bucket so the ansible aws_ssm connection can transfer files to this host."
  type        = string
}

variable "tags" {
  description = "Resource tags (merged with compliance default_tags by the root module)."
  type        = map(string)
  default     = {}
}
