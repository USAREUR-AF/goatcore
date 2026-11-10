# relay-health-host module — an always-on cloud host for the cloud-resident
# relay-health surface (ADR 1061). Cloned from the proven mesh-dns-host pattern
# (ADR 1056): Ubuntu 24.04, default VPC, SSM-managed (port 22 closed) so ansible
# reaches it without the operator being on the mesh, S3 grant for the aws_ssm
# file channel, mesh-joined via cloud-init, Docker + host-wide log bound.
#
# The relay-health surface (Traefik + oauth2-proxy + the dashboard + the ADR-1058
# ingest + an external-vantage prober) is deployed separately by ansible over
# SSM — exactly as CoreDNS is on the mesh-dns hosts.
#
# Two phases (var.public_enabled, default false):
#   PHASE 1 (public_enabled = false): egress-only SG, no EIP — no public surface
#     at all. The whole surface stands up + verifies mesh-internally / over SSM.
#   PHASE 2 (public_enabled = true): attach an Elastic IP + open :443 (+ :80 for
#     ACME HTTP-01 if used) — the single, deliberate, operator-watched public
#     flip. Cloudflare A-record + the Dex oauth2-proxy client are wired then.

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
  name = "relay-health-${var.goatnet}-${var.name}"
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
# SSM bucket explicitly (same bucket the relays + mesh-dns hosts use).
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
  name = "relay-health-${var.goatnet}-${var.name}"
  role = aws_iam_role.this.name
}

# Resource descriptions. Every one is an input: a security-group description
# is immutable (changing it replaces the group), so a root whose live goatnet
# carries other text passes it in `descriptions` and plans no change.
locals {
  descriptions = merge({
    security_group = "Cloud-resident relay-health host ${var.name} - egress-only in PHASE 1; :443 opened only when public_enabled (the gated PHASE 2 flip)"
    public_443     = "relay-health public 443 (PHASE 2)"
    public_80      = "relay-health public 80 (PHASE 2)"
  }, var.descriptions)
}

resource "aws_security_group" "this" {
  name        = "relay-health-${var.goatnet}-${var.name}"
  description = local.descriptions.security_group
  vpc_id      = data.aws_vpc.default.id

  # PHASE 2 public ingress — gated on public_enabled. :443 (Traefik/oauth2-proxy)
  # + :80 (ACME HTTP-01 fallback; DNS-01 via Cloudflare needs no :80). Closed in
  # PHASE 1 (no blocks rendered) = the no-public-exposure invariant.
  dynamic "ingress" {
    for_each = var.public_enabled ? [443, 80] : []
    content {
      from_port   = ingress.value
      to_port     = ingress.value
      protocol    = "tcp"
      cidr_blocks = var.public_ingress_cidrs
      description = local.descriptions["public_${ingress.value}"]
    }
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = merge(var.tags, { Name = "relay-health-${var.goatnet}-${var.name}" })
}

locals {
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    exec > >(tee /var/log/relay-health-bootstrap.log) 2>&1
    echo "[$(date -Is)] relay-health-${var.goatnet}-${var.name} bootstrap starting"

    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq ca-certificates curl gnupg jq

    # ---- Docker (official repo) — the surface runs as containers ----
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

    # ---- Mesh client: stock netbird, enroll on the ZAGROS mesh ----
    # Mesh membership lets us verify + reach the host pre-public-flip (Phase 1)
    # and gives the host a mesh identity; the PUBLIC break-glass path (Phase 2)
    # is independent of the mesh (that is the point).
    curl -fsSL https://pkgs.netbird.io/debian/public.key | gpg --dearmor -o /usr/share/keyrings/netbird-archive-keyring.gpg
    echo "deb [signed-by=/usr/share/keyrings/netbird-archive-keyring.gpg] https://pkgs.netbird.io/debian stable main" \
      > /etc/apt/sources.list.d/netbird.list
    apt-get update -qq
    apt-get install -y -qq netbird
    echo "[$(date -Is)] enrolling on ZAGROS mesh as relay-health-${var.goatnet}-${var.name}"
    for attempt in 1 2 3 4 5; do
      if netbird up --setup-key '${var.netbird_setup_key}' --management-url '${var.netbird_mgmt_url}' --hostname 'relay-health-${var.goatnet}-${var.name}'; then
        echo "[$(date -Is)] netbird up ok (attempt $attempt)"; break
      fi
      echo "[$(date -Is)] netbird up attempt $attempt failed; retrying in 20s"; sleep 20
    done || true

    echo "[$(date -Is)] bootstrap complete — relay-health surface deployed separately by ansible over SSM."
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
  #     DESTROY + recreate this long-lived host (losing the surface + enrollment).
  #   - user_data: carries the netbird setup-key, which ROTATES on its own
  #     cadence (keys expire every 14-90 days, minted fresh per deploy). Without
  #     this, an apply run after any key rotation sees changed user_data and,
  #     with user_data_replace_on_change, DESTROYS + recreates the enrolled host.
  #     The key is a one-time enrollment input — irrelevant once wt0 is up.
  lifecycle {
    ignore_changes = [ami, user_data]
  }

  tags = merge(var.tags, { Name = "relay-health-${var.goatnet}-${var.name}" })
}

# Elastic IP — only in PHASE 2 (stable public address for the Cloudflare record).
resource "aws_eip" "this" {
  count    = var.public_enabled ? 1 : 0
  instance = aws_instance.this.id
  domain   = "vpc"
  tags     = merge(var.tags, { Name = "relay-health-${var.goatnet}-${var.name}" })
}
