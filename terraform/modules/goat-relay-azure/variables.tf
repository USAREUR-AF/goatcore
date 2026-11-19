variable "relay_name" {
  description = "Short name for this relay (e.g. 'sto', 'fra'). Empty = primary (no suffix)."
  type        = string
  default     = ""
}

variable "location" {
  description = "Azure region, e.g. 'swedencentral'."
  type        = string
}

variable "zones" {
  description = "Availability zone for the VM and its public IP, e.g. [\"1\"]. Empty = no zone (for regions without zones)."
  type        = list(string)
  default     = ["1"]
}

variable "resource_group_name" {
  description = "Existing resource group the relay's resources go in."
  type        = string
}

variable "project" {
  description = "Project name prefix."
  type        = string
}

variable "environment" {
  description = "Environment name."
  type        = string
}

variable "service_name" {
  description = "Short service name."
  type        = string
  default     = "goat-relay"
}

variable "vm_size" {
  description = <<-EOT
    VM size. ADR 0712 sets an 8 vCPU floor for the 8-shard relay; the AWS module
    uses c7i.2xlarge (8 vCPU, compute-optimised). Standard_F8s_v2 is the
    nearest Azure match.
  EOT
  type        = string
  default     = "Standard_F8s_v2"
}

variable "admin_username" {
  description = "Admin user on the VM. 'ubuntu' matches the AWS image, so plays need no change."
  type        = string
  default     = "ubuntu"
}

variable "encryption_at_host_enabled" {
  description = "Encrypt the temp disk and caches on the host too. Needs the EncryptionAtHost feature registered on the subscription."
  type        = bool
  default     = false
}

variable "subnet_id" {
  description = "Existing subnet ID. Empty = create a standalone VNet and subnet."
  type        = string
  default     = ""
}

variable "vnet_cidr" {
  description = "Address space of the auto-created VNet. Must not overlap the Bastion VNet it is peered with."
  type        = string
  default     = "10.99.1.0/24"
}

variable "ssh_source_prefixes" {
  description = <<-EOT
    Prefixes allowed to reach port 22: the AzureBastionSubnet of the peered
    Bastion VNet. Default empty = no SSH rule, so the VM is unreachable for
    administration until Bastion is wired.
  EOT
  type        = list(string)
  default     = []
}

variable "ssh_public_key" {
  description = <<-EOT
    OpenSSH public key for the admin user. Empty = generate a key pair in Terraform and store the
    private key in key_vault_id; the private key is then also in the state. Set it when people
    outside the operators can read the state.
  EOT
  type        = string
  default     = ""
}

variable "key_vault_id" {
  description = "Key Vault for a generated SSH private key. Needed only when ssh_public_key is empty. The applying identity needs secret set/get on it."
  type        = string
  default     = ""
}

variable "dns_zone_name" {
  description = "Azure DNS zone the relay's A record goes in, e.g. 'goat.example'."
  type        = string
}

variable "dns_zone_resource_group_name" {
  description = "Resource group of the Azure DNS zone."
  type        = string
}

variable "relay_dns_prefix" {
  description = "Prefix of the per-relay record: <prefix>-<relay_name>.<zone>."
  type        = string
  default     = "goat-relay"
}

variable "wg_cp0_enabled" {
  description = "Open the wg-cp0 silent control-plane WireGuard listener (ADR 0313)."
  type        = bool
  default     = false
}

variable "wg_cp0_port" {
  description = "UDP port for the wg-cp0 listener."
  type        = number
  default     = 51821
}

variable "rathole_ingress_enabled" {
  description = "Open 443 (public enrolment) and 2333 (rathole control) for colocated rathole ingress, for a site behind CGNAT."
  type        = bool
  default     = false
}

variable "rathole_source_prefixes" {
  description = "Prefixes allowed to reach the rathole control channel (TCP 2333). Narrow to the site's egress address when known."
  type        = list(string)
  default     = ["0.0.0.0/0"]
}

variable "coturn_enabled" {
  description = "Open coturn STUN on 3478 UDP and TCP (F-192)."
  type        = bool
  default     = false
}

variable "coturn_turn_enabled" {
  description = "Also open the TURN media range UDP 49152-65535. Needs coturn_enabled."
  type        = bool
  default     = false
}

variable "tags" {
  description = "Extra tags for every resource."
  type        = map(string)
  default     = {}
}
