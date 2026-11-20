output "instance_id" {
  value = aws_instance.this.id
}

output "hostname" {
  value = local.host
}

output "private_ip" {
  value = aws_instance.this.private_ip
}

output "public_ip" {
  description = "The static Elastic IP the partner allowlists."
  value       = aws_eip.this.public_ip
}
