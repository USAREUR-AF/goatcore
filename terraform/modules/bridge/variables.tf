variable "bridge_name" {
  description = "Short name for this bridge (e.g., 'fra'). Used in resource names and DNS."
  type        = string
  default     = ""
}

variable "region" {
  description = "AWS region"
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
  default     = "bridge"
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
  default     = "10.98.0.0/24"
}

variable "ssh_cidr_blocks" {
  description = "CIDR blocks allowed for SSH"
  type        = list(string)
  default     = []
}

variable "forwarded_tcp_ports" {
  description = "TCP ports to allow inbound for forwarding (e.g., [9000, 8089]). Empty by default — no inbound access until explicitly configured."
  type        = list(number)
  default     = []
}

variable "forwarded_port_cidr_blocks" {
  description = "CIDR blocks allowed to reach forwarded TCP ports. Must be set to partner IP ranges — no default open access."
  type        = list(string)
  default     = []
}

variable "forwarded_port_ranges" {
  description = "Port ranges/individual ports to allow inbound, each with its own protocol and CIDR list. Use from == to for a single port. Independent of forwarded_tcp_ports/forwarded_port_cidr_blocks."
  type = list(object({
    from        = number
    to          = number
    protocol    = optional(string, "tcp")
    cidr_blocks = list(string)
  }))
  default = []
}

variable "cloudflare_zone_id" {
  description = "Cloudflare zone ID for DNS records"
  type        = string
}

variable "dns_zone_name" {
  description = "DNS zone the bridge_url output names the bridge under."
  type        = string
  default     = "legacy.example"
}
