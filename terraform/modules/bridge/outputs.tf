output "public_ip" {
  description = "Elastic IP of the bridge (give this to partners for whitelisting)"
  value       = aws_eip.bridge.public_ip
}

output "instance_id" {
  description = "EC2 instance ID"
  value       = aws_instance.bridge.id
}

output "bridge_url" {
  description = "DNS hostname for the bridge"
  value       = "${local.dns_name}.${var.dns_zone_name}"
}

output "ssh_key_secret_name" {
  description = "Secrets Manager name for SSH key"
  value       = aws_secretsmanager_secret.ssh_key.name
}

output "vpc_id" {
  description = "VPC ID (created or provided)"
  value       = local.vpc_id
}
