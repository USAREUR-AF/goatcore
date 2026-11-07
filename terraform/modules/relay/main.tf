# modules/relay — Single VPS relay instance
#
# Creates an EC2 instance running rathole, with Elastic IP, security group,
# IAM role (SSM), and SSH key in Secrets Manager. Optionally creates a
# standalone VPC if vpc_id is not provided.

terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4.0"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.0"
    }
  }
}

# -- Locals -------------------------------------------------------------------

locals {
  create_vpc    = var.vpc_id == ""
  vpc_id        = local.create_vpc ? aws_vpc.relay[0].id : var.vpc_id
  subnet_id     = local.create_vpc ? aws_subnet.relay[0].id : var.public_subnet_id
  relay_suffix  = var.relay_name != "" ? "-${var.relay_name}" : ""
  resource_name = "${var.project}-${var.environment}-${var.service_name}${local.relay_suffix}"
  dns_name      = var.relay_name != "" ? "${var.relay_dns_prefix}-${var.relay_name}" : var.relay_dns_prefix
}

# -- VPC (created only when vpc_id is not provided) ---------------------------

resource "aws_vpc" "relay" {
  count      = local.create_vpc ? 1 : 0
  cidr_block = var.vpc_cidr

  tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
}

resource "aws_internet_gateway" "relay" {
  count  = local.create_vpc ? 1 : 0
  vpc_id = aws_vpc.relay[0].id

  tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
}

resource "aws_subnet" "relay" {
  count                   = local.create_vpc ? 1 : 0
  vpc_id                  = aws_vpc.relay[0].id
  cidr_block              = var.vpc_cidr
  map_public_ip_on_launch = true
  availability_zone       = "${var.region}a"

  tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
}

resource "aws_route_table" "relay" {
  count  = local.create_vpc ? 1 : 0
  vpc_id = aws_vpc.relay[0].id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = aws_internet_gateway.relay[0].id
  }

  tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
}

resource "aws_route_table_association" "relay" {
  count          = local.create_vpc ? 1 : 0
  subnet_id      = aws_subnet.relay[0].id
  route_table_id = aws_route_table.relay[0].id
}

# -- SSH Key Pair --------------------------------------------------------------

resource "tls_private_key" "ssh" {
  algorithm = "RSA"
  rsa_bits  = 4096
}

resource "aws_key_pair" "main" {
  key_name   = local.resource_name
  public_key = tls_private_key.ssh.public_key_openssh
  tags       = { Service = var.service_name }
}

resource "random_id" "secret_suffix" {
  byte_length = 4
}

resource "aws_secretsmanager_secret" "ssh_key" {
  name        = "${local.resource_name}-ssh-${random_id.secret_suffix.hex}"
  description = "SSH private key for ${local.resource_name}"
  tags        = { Service = var.service_name }
}

resource "aws_secretsmanager_secret_version" "ssh_key" {
  secret_id     = aws_secretsmanager_secret.ssh_key.id
  secret_string = tls_private_key.ssh.private_key_pem
}

# -- IAM Role (SSM) -----------------------------------------------------------

resource "aws_iam_role" "ec2" {
  name = local.resource_name

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Action    = "sts:AssumeRole"
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
    }]
  })

  tags = { Service = var.service_name }
}

resource "aws_iam_role_policy_attachment" "ssm" {
  role       = aws_iam_role.ec2.name
  policy_arn = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
}

resource "aws_iam_instance_profile" "ec2" {
  name = local.resource_name
  role = aws_iam_role.ec2.name
}

# -- Security Group ------------------------------------------------------------

resource "aws_security_group" "main" {
  name        = local.resource_name
  description = "VPS relay: rathole (TCP 443, UDP 3478, TCP 2333) + SSH"
  vpc_id      = local.vpc_id

  # SSH: no public access by default. Use SSM Session Manager for shell access:
  #   aws ssm start-session --target <instance-id>
  # If direct SSH is needed, pass ssh_cidr_blocks explicitly.
  dynamic "ingress" {
    for_each = length(var.ssh_cidr_blocks) > 0 ? [1] : []
    content {
      from_port   = 22
      to_port     = 22
      protocol    = "tcp"
      cidr_blocks = var.ssh_cidr_blocks
      description = "SSH (restricted)"
    }
  }

  # HTTP/80 — historically opened for ACME HTTP-01 challenge. The relays
  # use Cloudflare DNS-01 ACME (Traefik `le` resolver), so 80/tcp is
  # unused by the cert chain. Defaults to open for backwards compatibility
  # (terraform/ + terraform-prod/ trees); a site can set `http_port_open
  # = false` to drop the scannable surface. See
  # the public-posture design doc.
  dynamic "ingress" {
    for_each = var.http_port_open ? [1] : []
    content {
      from_port   = 80
      to_port     = 80
      protocol    = "tcp"
      cidr_blocks = ["0.0.0.0/0"]
      description = "HTTP - ACME HTTP-01 challenge (unused with DNS-01)"
    }
  }

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "HTTPS / Netbird management relay"
  }

  ingress {
    from_port   = 3478
    to_port     = 3478
    protocol    = "udp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "STUN/TURN relay"
  }

  ingress {
    from_port   = 2333
    to_port     = 2333
    protocol    = "tcp"
    cidr_blocks = var.rathole_cidr_blocks
    description = "rathole control channel"
  }

  ingress {
    from_port   = 33073
    to_port     = 33073
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "Netbird relay WebSocket data relay"
  }

  ingress {
    from_port   = 33073
    to_port     = 33073
    protocol    = "udp"
    cidr_blocks = ["0.0.0.0/0"]
    description = var.quic_rule_description
  }

  # DNS (UDP 53) is NOT exposed publicly. dnsmasq on relays serves mesh-internal
  # DNS only — peers reach it via the WireGuard tunnel (Netbird DNS nameserver group).
  # No security group rule needed; mesh traffic arrives on the WireGuard interface.

  ingress {
    from_port   = 49152
    to_port     = 65535
    protocol    = "udp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "Coturn TURN relay ports"
  }

  # Block 22 — silent control plane via outer WireGuard wrap (ADR 0313).
  # wg-cp0 listener on UDP 51821 (configurable). Silent under unauthenticated
  # handshakes by WireGuard's protocol property — packets without a valid
  # peer key get cryptographically dropped in-kernel, no response, no
  # fingerprintable banner. Required by terraform-prod/; legacy tree
  # leaves wg_cp0_enabled = false (legacy rathole pattern stays).
  dynamic "ingress" {
    for_each = var.wg_cp0_enabled ? [1] : []
    content {
      from_port   = var.wg_cp0_port
      to_port     = var.wg_cp0_port
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = "wg-cp0 silent control-plane WireGuard listener (Block 22)"
    }
  }

  ingress {
    from_port   = -1
    to_port     = -1
    protocol    = "icmp"
    cidr_blocks = ["0.0.0.0/0"]
    description = var.icmp_rule_description
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
    description = "All outbound"
  }

  tags = { Service = var.service_name }
}

# -- EC2 Instance (x86_64 Ubuntu 24.04) ----------------------------------------

data "aws_ami" "ubuntu_x86" {
  most_recent = true
  owners      = ["099720109477"]

  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*"]
  }

  filter {
    name   = "virtualization-type"
    values = ["hvm"]
  }
}

resource "aws_instance" "relay" {
  ami                    = data.aws_ami.ubuntu_x86.id
  instance_type          = var.instance_type
  iam_instance_profile   = aws_iam_instance_profile.ec2.name
  vpc_security_group_ids = [aws_security_group.main.id]
  subnet_id              = local.subnet_id
  key_name               = aws_key_pair.main.key_name

  root_block_device {
    volume_size           = 20
    volume_type           = "gp3"
    encrypted             = true
    delete_on_termination = true
  }

  metadata_options {
    http_endpoint = "enabled"
    http_tokens   = "required"
  }

  tags = {
    Name    = local.resource_name
    Service = var.service_name
    Region  = var.region
    Purpose = var.purpose_tag
  }

  lifecycle {
    ignore_changes = [ami]
  }
}

# -- Elastic IP (stable public address) ----------------------------------------

resource "aws_eip" "relay" {
  instance = aws_instance.relay.id
  domain   = "vpc"

  tags = {
    Name    = local.resource_name
    Service = var.service_name
  }
}

# -- Rathole credential storage ------------------------------------------------

resource "aws_secretsmanager_secret" "rathole_credentials" {
  name        = "${local.resource_name}-rathole-${random_id.secret_suffix.hex}"
  description = "rathole relay credentials for ${local.resource_name} (token + public key)"
  tags        = { Service = var.service_name }
}

# -- Cloudflare DNS -------------------------------------------------------------

resource "cloudflare_record" "relay" {
  zone_id = var.cloudflare_zone_id
  name    = local.dns_name
  content = aws_eip.relay.public_ip
  type    = "A"
  proxied = false
}

resource "cloudflare_record" "management" {
  count   = var.management_dns ? 1 : 0
  zone_id = var.cloudflare_zone_id
  name    = var.management_dns_name
  content = aws_eip.relay.public_ip
  type    = "A"
  ttl     = 60
  proxied = false
}

resource "cloudflare_record" "status" {
  count   = var.status_dns ? 1 : 0
  zone_id = var.cloudflare_zone_id
  name    = var.status_dns_name
  content = aws_eip.relay.public_ip
  type    = "A"
  ttl     = 60
  proxied = false
}
