output "instance_id" {
  value = aws_instance.this.id
}

output "hostname" {
  value = "mesh-dns-${var.goatnet}-${var.name}"
}

output "private_ip" {
  value = aws_instance.this.private_ip
}

output "az" {
  value = aws_instance.this.availability_zone
}
