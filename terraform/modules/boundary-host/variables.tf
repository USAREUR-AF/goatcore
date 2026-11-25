variable "prefix" {
  description = "Host name prefix; the host is <prefix>-<goatnet>-<name>."
  type        = string
  default     = "simple-bn"
}

variable "name" {
  description = "Short instance name, e.g. a."
  type        = string
  default     = "a"
}

variable "goatnet" {
  description = "The goatnet this host belongs to; part of every resource name."
  type        = string
}

variable "az" {
  type    = string
  default = "eu-central-1a"
}

variable "instance_type" {
  type    = string
  default = "t3.small"
}

variable "root_volume_size" {
  type    = number
  default = 16
}

variable "netbird_setup_key" {
  description = "One-off NetBird setup key for enrollment. Sensitive; used once."
  type        = string
  sensitive   = true
}

variable "netbird_mgmt_url" {
  description = "The goatnet's NetBird management URL."
  type        = string
}

variable "ssm_bucket_arn" {
  description = "The site SSM bucket the ansible aws_ssm connection stages files through."
  type        = string
}

variable "ingress" {
  description = "Inbound rules, each limited to a partner's addresses: [{ port, protocol = udp | tcp, cidrs, description }]."
  type = list(object({
    port        = number
    protocol    = string
    cidrs       = list(string)
    description = string
  }))
  default = []

  validation {
    condition = alltrue([for r in var.ingress :
      contains(["udp", "tcp"], r.protocol) && r.port >= 1 && r.port <= 65535 &&
      length(r.cidrs) > 0 && !contains(r.cidrs, "0.0.0.0/0")
    ])
    error_message = "ingress: protocol udp or tcp, port 1-65535, and at least one CIDR that is not 0.0.0.0/0."
  }
}

variable "ipsec_gateway_cidrs" {
  description = "Encryption stage 2 (IPsec): the receiver's gateway addresses, admitted on UDP 500 (IKE) and UDP 4500 (NAT-T, which carries ESP behind the host's 1:1 NAT). Empty = no rule. For a responder whose initiator has no fixed address: the initiator's NAT egress /32 if they can name it, else 0.0.0.0/0 (the role then admits only the declared remote_id with the key)."
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for c in var.ipsec_gateway_cidrs : can(cidrnetmask(c))])
    error_message = "ipsec_gateway_cidrs: IPv4 CIDRs (a gateway is usually one /32)."
  }
}

variable "tags" {
  type    = map(string)
  default = {}
}
