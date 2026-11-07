# Offline plans against mocked providers; no Azure credentials needed.
#   terraform init -backend=false && terraform test

mock_provider "azurerm" {}
mock_provider "tls" {}

variables {
  location                     = "westeurope"
  resource_group_name          = "rg-test"
  project                      = "test"
  environment                  = "dev"
  relay_name                   = "a"
  dns_zone_name                = "goat.example"
  dns_zone_resource_group_name = "rg-dns"
  ssh_public_key               = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA test" # opsec-allow: synthetic public key for mocked Terraform plans
}

run "external_key_generates_nothing" {
  command = plan
  assert {
    condition     = length(tls_private_key.ssh) == 0 && length(azurerm_key_vault_secret.ssh_key) == 0
    error_message = "With ssh_public_key set, no key may be generated or stored."
  }
}

run "generated_key_needs_a_vault" {
  command = plan
  variables {
    ssh_public_key = ""
  }
  expect_failures = [tls_private_key.ssh]
}

run "caller_tag_replaces_default_case_insensitively" {
  command = plan
  variables {
    tags = { service = "from-policy" }
  }
  assert {
    condition     = azurerm_public_ip.relay.tags["service"] == "from-policy" && !contains(keys(azurerm_public_ip.relay.tags), "Service")
    error_message = "A caller's service tag must replace the default Service tag."
  }
}

run "nsg_opens_quic_and_websocket_only_by_default" {
  command = plan
  assert {
    condition = toset([for r in azurerm_network_security_group.relay.security_rule : r.name]) == toset([
      "goat-relay-ws", "goat-relay-quic", "icmp", "deny-all-inbound",
    ])
    error_message = "Default inbound surface must be 33073 TCP+UDP, ICMP and the explicit deny."
  }
}

run "any_source_uses_the_single_prefix_field" {
  command = plan
  variables {
    rathole_ingress_enabled = true
    rathole_source_prefixes = ["192.0.2.10/32"]
  }
  assert {
    condition = alltrue([
      for r in azurerm_network_security_group.relay.security_rule :
      r.name == "rathole-control" ? (r.source_address_prefixes == toset(["192.0.2.10/32"]) && r.source_address_prefix == null) : true
    ])
    error_message = "The rathole control port must admit only the site's egress prefixes."
  }
  assert {
    condition = alltrue([
      for r in azurerm_network_security_group.relay.security_rule :
      contains(["goat-relay-ws", "goat-relay-quic", "rathole-enroll", "icmp"], r.name) ? (r.source_address_prefix == "*" && try(length(r.source_address_prefixes), 0) == 0) : true
    ])
    error_message = "Any-source rules must use source_address_prefix = \"*\", never a \"*\" inside the prefix list."
  }
}
