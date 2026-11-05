output "public_ip" {
  description = "Static public IP of the relay. The root adds it to the round-robin record set."
  value       = azurerm_public_ip.relay.ip_address
}

output "vm_id" {
  description = "Azure resource ID of the VM: the Bastion target for `az network bastion ssh|tunnel --target-resource-id`."
  value       = azurerm_linux_virtual_machine.relay.id
}

output "relay_url" {
  description = "Full DNS hostname of the relay."
  value       = "${local.dns_name}.${var.dns_zone_name}"
}

output "admin_username" {
  description = "Admin user on the VM."
  value       = var.admin_username
}

output "ssh_key_secret_name" {
  description = "Key Vault secret holding the generated SSH private key. Empty when ssh_public_key was passed in."
  value       = local.generate_key ? azurerm_key_vault_secret.ssh_key[0].name : ""
}

output "rathole_secret_name" {
  description = "Key Vault secret name the rathole deploy writes its credentials to (empty when rathole_ingress_enabled = false). Not created here: Key Vault has no empty secrets."
  value       = var.rathole_ingress_enabled ? "${local.secret_base}-rathole" : ""
}

output "vnet_id" {
  description = "ID of the created VNet, for peering with the Bastion VNet. Empty when subnet_id was passed in."
  value       = local.create_vnet ? azurerm_virtual_network.relay[0].id : ""
}

output "vnet_name" {
  description = "Name of the created VNet, for the peering back to the Bastion VNet. Empty when subnet_id was passed in."
  value       = local.create_vnet ? azurerm_virtual_network.relay[0].name : ""
}

output "identity_principal_id" {
  description = "Object ID of the VM's system-assigned identity, for role assignments such as DNS Zone Contributor."
  value       = azurerm_linux_virtual_machine.relay.identity[0].principal_id
}
