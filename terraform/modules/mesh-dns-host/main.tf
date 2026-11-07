# mesh-dns-host module — see variables.tf for the role of this host.
#
# Posture (mirrors the moon-pod-test enroll pattern + the relay SSM posture):
#   - SSM-managed (AmazonSSMManagedInstanceCore) so ansible reaches it with
#     port 22 closed — the operator does not need to be on the mesh.
#   - S3 read/write on the site SSM bucket so the ansible aws_ssm file-transfer
#     channel works (the managed-core policy does not grant arbitrary S3).
#   - Egress-only SG: no public ingress. DNS is served to peers over the mesh
#     (WireGuard), never the public internet — the no-public-exposure invariant.
#   - cloud-init installs Docker + netbird and enrolls on the mesh.

terraform {
  required_providers {
    aws = { source = "hashicorp/aws" }
  }
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
  name = "mesh-dns-${var.goatnet}-${var.name}"
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

# The ansible aws_ssm connection plugin stages files through an S3 bucket; the
# managed-core policy does not grant it, so attach scoped access to the site
# SSM bucket explicitly (same bucket the relays use).
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
  name = "mesh-dns-${var.goatnet}-${var.name}"
  role = aws_iam_role.this.name
}

resource "aws_security_group" "this" {
  name        = "mesh-dns-${var.goatnet}-${var.name}"
  description = "${local.label} mesh CoreDNS replica ${var.name} - egress-only; DNS served over the mesh, no public ingress"
  vpc_id      = data.aws_vpc.default.id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = merge(var.tags, { Name = "mesh-dns-${var.goatnet}-${var.name}" })
}

locals {
  # Display label in descriptions and boot log lines; the goatnet id upper-cased
  # unless the caller names one (a description change replaces the security group).
  label     = coalesce(var.label, upper(var.goatnet))
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    exec > >(tee /var/log/mesh-dns-bootstrap.log) 2>&1
    echo "[$(date -Is)] mesh-dns-${var.goatnet}-${var.name} bootstrap starting"

    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq ca-certificates curl gnupg jq

    # ---- Docker (official repo) — CoreDNS runs as a container ----
    install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    chmod a+r /etc/apt/keyrings/docker.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo $VERSION_CODENAME) stable" \
      > /etc/apt/sources.list.d/docker.list
    apt-get update -qq
    apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin
    systemctl enable --now docker

    # ---- Host-wide log bound (the F-229 lesson: bound EVERY container's log) ----
    install -d -m 0755 /etc/docker
    cat > /etc/docker/daemon.json <<'JSON'
    { "log-driver": "json-file", "log-opts": { "max-size": "50m", "max-file": "5" } }
    JSON
    systemctl restart docker

    # ---- Mesh client: stock netbird, enroll on the ${local.label} mesh ----
    curl -fsSL https://pkgs.netbird.io/debian/public.key | gpg --dearmor -o /usr/share/keyrings/netbird-archive-keyring.gpg
    echo "deb [signed-by=/usr/share/keyrings/netbird-archive-keyring.gpg] https://pkgs.netbird.io/debian stable main" \
      > /etc/apt/sources.list.d/netbird.list
    apt-get update -qq
    apt-get install -y -qq netbird
    echo "[$(date -Is)] enrolling on ${local.label} mesh as mesh-dns-${var.goatnet}-${var.name}"
    for attempt in 1 2 3 4 5; do
      if netbird up --setup-key '${var.netbird_setup_key}' --management-url '${var.netbird_mgmt_url}' --hostname 'mesh-dns-${var.goatnet}-${var.name}'; then
        echo "[$(date -Is)] netbird up ok (attempt $attempt)"; break
      fi
      echo "[$(date -Is)] netbird up attempt $attempt failed; retrying in 20s"; sleep 20
    done || true

    echo "[$(date -Is)] bootstrap complete — CoreDNS deployed separately by ops/ansible/goat-coredns.yml over SSM."
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
    http_put_response_hop_limit = 2          # docker containers may reach IMDS
  }

  root_block_device {
    volume_size = var.root_volume_size
    volume_type = "gp3"
    encrypted   = true
  }

  # Pin BOTH the AMI and the rendered user_data against incidental drift — this
  # is a long-lived host; re-image deliberately (taint / -replace) when an OS
  # bump or a real bootstrap change is wanted, never as a side effect of a
  # routine apply.
  #   - ami: most_recent lookup drifts every time Canonical publishes a new
  #     Ubuntu image; without this, any apply touching the module would
  #     DESTROY + recreate this host (losing the mesh enrollment / DNS surface).
  #   - user_data: carries the netbird setup-key, which ROTATES on its own
  #     cadence (keys expire every 14-90 days, minted fresh per deploy). Without
  #     this, an apply run after any key rotation sees changed user_data and,
  #     with user_data_replace_on_change, DESTROYS + recreates the enrolled host.
  #     The key is a one-time enrollment input — irrelevant once wt0 is up.
  lifecycle {
    ignore_changes = [ami, user_data]
  }

  tags = merge(var.tags, { Name = "mesh-dns-${var.goatnet}-${var.name}" })
}
