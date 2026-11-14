# modules/goat-relay-azure — one goat-relay VM on Azure.
#
# The Azure counterpart of modules/goat-relay, for a goatnet whose cloud must
# not be AWS. Same relay, same ports, same public DNS name; the differences:
#
# - Admin access is Azure Bastion, not SSM. Port 22 is closed to the internet
#   and open only to the source prefixes passed in ssh_source_prefixes (the
#   Bastion subnet). Bastion lives in the root, not here: one Bastion serves
#   every relay through VNet peering, and this module outputs the VNet to peer.
# - The SSH private key is a Key Vault secret instead of a Secrets Manager
#   secret. The vault belongs to the root and is passed in. Or pass
#   ssh_public_key: the key pair is then made outside Terraform and no private
#   key ever enters the state (recommended where others can read the state).
# - DNS is an Azure DNS A record. The round-robin pool is NOT created here:
#   an Azure record set is one resource holding every address, so the root
#   builds it from all relays' public_ip outputs.
# - No rathole credentials container. AWS allows an empty secret; Key Vault
#   does not. rathole_secret_name is the name the deploy play writes to.
# - No A/B stock-relay experiment port. It was a temporary surface.
#
# Status: designed and validated with `terraform validate`; not yet applied.

terraform {
  required_version = ">= 1.5"
  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 4.0"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
  }
}

# -- Locals -------------------------------------------------------------------

locals {
  generate_key  = var.ssh_public_key == ""
  create_vnet   = var.subnet_id == ""
  subnet_id     = local.create_vnet ? azurerm_subnet.relay[0].id : var.subnet_id
  relay_suffix  = var.relay_name != "" ? "-${var.relay_name}" : ""
  resource_name = "${var.project}-${var.environment}-${var.service_name}${local.relay_suffix}"
  dns_name      = var.relay_name != "" ? "${var.relay_dns_prefix}-${var.relay_name}" : var.relay_dns_prefix
  # Key Vault secret names allow only letters, digits and dashes.
  secret_base = replace(local.resource_name, "/[^0-9A-Za-z-]/", "-")
  # Defaults first; a caller's tag of the same name wins, compared case-insensitively because
  # Azure tag names are (e.g. an inherited policy tag `service` replaces the default `Service`).
  default_tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
  caller_tag_names = [for k in keys(var.tags) : lower(k)]
  tags = merge(
    { for k, v in local.default_tags : k => v if !contains(local.caller_tag_names, lower(k)) },
    var.tags,
  )

  # Inbound rules. Priorities are fixed per rule so a plan never reshuffles
  # them; a disabled rule is simply absent.
  inbound_rules = concat(
    [
      { name = "goat-relay-ws", priority = 100, protocol = "Tcp", port = "33073", sources = ["*"] },
      # F-021: the QUIC path needs UDP as well; TCP alone is not enough.
      { name = "goat-relay-quic", priority = 110, protocol = "Udp", port = "33073", sources = ["*"] },
      { name = "icmp", priority = 900, protocol = "Icmp", port = "*", sources = ["*"] },
    ],
    length(var.ssh_source_prefixes) > 0 ? [
      { name = "ssh-bastion", priority = 200, protocol = "Tcp", port = "22", sources = var.ssh_source_prefixes },
    ] : [],
    var.wg_cp0_enabled ? [
      { name = "wg-cp0", priority = 300, protocol = "Udp", port = tostring(var.wg_cp0_port), sources = ["*"] },
    ] : [],
    var.rathole_ingress_enabled ? [
      { name = "rathole-enroll", priority = 400, protocol = "Tcp", port = "443", sources = ["*"] },
      { name = "rathole-control", priority = 410, protocol = "Tcp", port = "2333", sources = var.rathole_source_prefixes },
    ] : [],
    var.coturn_enabled ? [
      { name = "coturn-stun-udp", priority = 500, protocol = "Udp", port = "3478", sources = ["*"] },
      { name = "coturn-stun-tcp", priority = 510, protocol = "Tcp", port = "3478", sources = ["*"] },
    ] : [],
    var.coturn_enabled && var.coturn_turn_enabled ? [
      { name = "coturn-turn-media", priority = 520, protocol = "Udp", port = "49152-65535", sources = ["*"] },
    ] : [],
  )
}

# -- Network (created only when subnet_id is not provided) --------------------

resource "azurerm_virtual_network" "relay" {
  count               = local.create_vnet ? 1 : 0
  name                = local.resource_name
  location            = var.location
  resource_group_name = var.resource_group_name
  address_space       = [var.vnet_cidr]
  tags                = local.tags
}

resource "azurerm_subnet" "relay" {
  count                = local.create_vnet ? 1 : 0
  name                 = "relay"
  resource_group_name  = var.resource_group_name
  virtual_network_name = azurerm_virtual_network.relay[0].name
  address_prefixes     = [var.vnet_cidr]
}

resource "azurerm_network_security_group" "relay" {
  name                = local.resource_name
  location            = var.location
  resource_group_name = var.resource_group_name
  tags                = local.tags

  dynamic "security_rule" {
    for_each = local.inbound_rules
    content {
      name                   = security_rule.value.name
      priority               = security_rule.value.priority
      direction              = "Inbound"
      access                 = "Allow"
      protocol               = security_rule.value.protocol
      source_port_range      = "*"
      destination_port_range = security_rule.value.port
      # "Any source" goes in the single-prefix field: Azure rejects "*" inside the prefix list.
      # Compared element-wise, since a list never equals a tuple literal in Terraform.
      source_address_prefixes    = length(security_rule.value.sources) == 1 && security_rule.value.sources[0] == "*" ? null : security_rule.value.sources
      source_address_prefix      = length(security_rule.value.sources) == 1 && security_rule.value.sources[0] == "*" ? "*" : null
      destination_address_prefix = "*"
    }
  }

  # Azure's default rules would admit everything from inside the VNet. Deny it
  # explicitly so the rules above are the whole inbound surface.
  security_rule {
    name                       = "deny-all-inbound"
    priority                   = 4000
    direction                  = "Inbound"
    access                     = "Deny"
    protocol                   = "*"
    source_port_range          = "*"
    destination_port_range     = "*"
    source_address_prefix      = "*"
    destination_address_prefix = "*"
  }
}

# -- SSH key, held in the root's Key Vault ------------------------------------

resource "tls_private_key" "ssh" {
  count     = local.generate_key ? 1 : 0
  algorithm = "ED25519"

  lifecycle {
    precondition {
      condition     = var.key_vault_id != ""
      error_message = "A generated key must be stored: pass key_vault_id, or pass ssh_public_key instead."
    }
  }
}

resource "azurerm_key_vault_secret" "ssh_key" {
  count        = local.generate_key ? 1 : 0
  name         = "${local.secret_base}-ssh"
  value        = tls_private_key.ssh[0].private_key_openssh
  key_vault_id = var.key_vault_id
  content_type = "openssh-private-key"
  tags         = local.tags
}

# -- Public address -----------------------------------------------------------

resource "azurerm_public_ip" "relay" {
  name                = local.resource_name
  location            = var.location
  resource_group_name = var.resource_group_name
  allocation_method   = "Static"
  sku                 = "Standard"
  zones               = var.zones
  tags                = local.tags
}

resource "azurerm_network_interface" "relay" {
  name                = local.resource_name
  location            = var.location
  resource_group_name = var.resource_group_name
  tags                = local.tags

  ip_configuration {
    name                          = "primary"
    subnet_id                     = local.subnet_id
    private_ip_address_allocation = "Dynamic"
    public_ip_address_id          = azurerm_public_ip.relay.id
  }
}

resource "azurerm_network_interface_security_group_association" "relay" {
  network_interface_id      = azurerm_network_interface.relay.id
  network_security_group_id = azurerm_network_security_group.relay.id
}

# -- VM (x86_64 Ubuntu 24.04) -------------------------------------------------

resource "azurerm_linux_virtual_machine" "relay" {
  name                            = local.resource_name
  location                        = var.location
  resource_group_name             = var.resource_group_name
  size                            = var.vm_size
  zone                            = length(var.zones) > 0 ? var.zones[0] : null
  admin_username                  = var.admin_username
  disable_password_authentication = true
  network_interface_ids           = [azurerm_network_interface.relay.id]
  encryption_at_host_enabled      = var.encryption_at_host_enabled
  secure_boot_enabled             = true
  vtpm_enabled                    = true

  # The relay's own Entra identity. The root grants it DNS Zone Contributor so
  # the relay renews its certificate by DNS-01 with no stored credential
  # (ops/ansible/tasks/relay-dns01-azure.yml).
  identity {
    type = "SystemAssigned"
  }
  tags = merge(local.tags, {
    Purpose = "goat-relay data-plane VM (ADR 0712 sizing: 8 vCPU floor)"
  })

  admin_ssh_key {
    username   = var.admin_username
    public_key = local.generate_key ? tls_private_key.ssh[0].public_key_openssh : var.ssh_public_key
  }

  os_disk {
    name                 = local.resource_name
    caching              = "ReadWrite"
    storage_account_type = "Premium_LRS"
    disk_size_gb         = 32
  }

  source_image_reference {
    publisher = "Canonical"
    offer     = "ubuntu-24_04-lts"
    sku       = "server"
    version   = "latest"
  }

  # Platform-managed encryption at rest is always on in Azure; host
  # encryption (temp disk and caches) is the opt-in above.

  lifecycle {
    ignore_changes = [source_image_reference[0].version]
  }
}

# -- Azure DNS ----------------------------------------------------------------

resource "azurerm_dns_a_record" "relay" {
  name                = local.dns_name
  zone_name           = var.dns_zone_name
  resource_group_name = var.dns_zone_resource_group_name
  ttl                 = 300
  records             = [azurerm_public_ip.relay.ip_address]
  tags                = local.tags
}
