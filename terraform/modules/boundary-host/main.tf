# boundary-host module — a dedicated cloud host for one simple boundary node or egress
# relay (ADR 1253, docs/design/simple-boundary-node.md). It runs nothing else: a boundary
# host is never shared with another service, so its failures, exposure and teardown are
# its own.
#
# Ubuntu 24.04 in the default VPC, managed over SSM (port 22 closed), joined to the
# goatnet's NetBird mesh by cloud-init, with a static Elastic IP: the address a partner
# receiver allowlists once. Inbound is closed except the rules in var.ingress, each
# limited to the partner's addresses, and, for encryption stage 2, UDP 500 and 4500 from
# var.ipsec_gateway_cidrs. The simple-bn itself is installed by
# ops/ansible/simple-bn.yml over SSM.

terraform {
  required_providers {
    aws = { source = "hashicorp/aws" }
  }
}

locals {
  host = "${var.prefix}-${var.goatnet}-${var.name}"
}

data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"] # Canonical
  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-*"]
  }
  filter {
    name   = "virtualization-type"
    values = ["hvm"]
  }
}

data "aws_vpc" "default" {
  default = true
}

resource "aws_iam_role" "this" {
  name = local.host
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
      Action    = "sts:AssumeRole"
    }]
  })
  tags = var.tags
}

resource "aws_iam_role_policy_attachment" "ssm" {
  role       = aws_iam_role.this.name
  policy_arn = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
}

# The ansible aws_ssm connection stages files through the site SSM bucket.
resource "aws_iam_role_policy" "ssm_bucket" {
  name = "ssm-bucket-access"
  role = aws_iam_role.this.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["s3:GetObject", "s3:PutObject", "s3:ListBucket", "s3:DeleteObject"]
      Resource = [var.ssm_bucket_arn, "${var.ssm_bucket_arn}/*"]
    }]
  })
}

resource "aws_iam_instance_profile" "this" {
  name = local.host
  role = aws_iam_role.this.name
}

resource "aws_security_group" "this" {
  name        = local.host
  description = "Boundary host ${local.host}: inbound only from the declared partner addresses"
  vpc_id      = data.aws_vpc.default.id

  dynamic "ingress" {
    for_each = var.ingress
    content {
      from_port   = ingress.value.port
      to_port     = ingress.value.port
      protocol    = ingress.value.protocol
      cidr_blocks = ingress.value.cidrs
      description = ingress.value.description
    }
  }

  # Encryption stage 2: IKE and NAT-T from the receiver's IPsec gateway only. Behind 1:1 NAT
  # strongSwan moves ESP into UDP 4500, so no ESP (protocol 50) rule is needed.
  dynamic "ingress" {
    for_each = length(var.ipsec_gateway_cidrs) > 0 ? [500, 4500] : []
    content {
      from_port   = ingress.value
      to_port     = ingress.value
      protocol    = "udp"
      cidr_blocks = var.ipsec_gateway_cidrs
      description = ingress.value == 500 ? "IPsec IKE from the receiver gateway" : "IPsec NAT-T from the receiver gateway"
    }
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = merge(var.tags, { Name = local.host })
}

locals {
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    exec > >(tee /var/log/boundary-host-bootstrap.log) 2>&1
    echo "[$(date -Is)] ${local.host} bootstrap starting"
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq ca-certificates curl gnupg jq python3
    curl -fsSL https://pkgs.netbird.io/debian/public.key | gpg --dearmor -o /usr/share/keyrings/netbird-archive-keyring.gpg
    echo "deb [signed-by=/usr/share/keyrings/netbird-archive-keyring.gpg] https://pkgs.netbird.io/debian stable main" \
      > /etc/apt/sources.list.d/netbird.list
    apt-get update -qq
    apt-get install -y -qq netbird
    for attempt in 1 2 3 4 5; do
      if netbird up --setup-key '${var.netbird_setup_key}' --management-url '${var.netbird_mgmt_url}' --hostname '${local.host}'; then
        echo "[$(date -Is)] netbird up ok (attempt $attempt)"; break
      fi
      echo "[$(date -Is)] netbird up attempt $attempt failed; retrying in 20s"; sleep 20
    done || true
    echo "[$(date -Is)] bootstrap complete; the simple-bn is installed by ansible over SSM."
  EOT
}

resource "aws_instance" "this" {
  ami                         = data.aws_ami.ubuntu.id
  instance_type               = var.instance_type
  availability_zone           = var.az
  iam_instance_profile        = aws_iam_instance_profile.this.name
  vpc_security_group_ids      = [aws_security_group.this.id]
  user_data                   = local.user_data
  user_data_replace_on_change = true

  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required" # IMDSv2 only
    http_put_response_hop_limit = 1
  }

  root_block_device {
    volume_size = var.root_volume_size
    volume_type = "gp3"
    encrypted   = true
  }

  # A long-lived host: re-image deliberately (-replace), never because Canonical
  # published a new AMI or the one-time enrollment key changed.
  lifecycle {
    ignore_changes = [ami, user_data]
  }

  tags = merge(var.tags, { Name = local.host })
}

# The static public address the partner allowlists. It survives instance replacement.
resource "aws_eip" "this" {
  domain = "vpc"
  tags   = merge(var.tags, { Name = local.host })
}

resource "aws_eip_association" "this" {
  instance_id   = aws_instance.this.id
  allocation_id = aws_eip.this.id
}
