# modules/goat-relay — Single VPS goat-relay instance (Block 33 / ADR 0712)
#
# Mirror of modules/relay shaped for the Rust goat-relay binary instead of
# stock netbird-relay. Differences from modules/relay:
#
# - instance_type default = c7i.2xlarge (8 vCPU = the architectural floor
#   for the 8-shard design per ADR 0709 D11 + Block 33C.1 lever L8). See
#   ADR 0712 for the sizing rationale.
# - Security group exposes the goat-relay data plane: TCP 33073 + UDP 33073
#   (per F-021 — both TCP and UDP must be open; AWS SG defaults plus Docker
#   -p shorthand each default to TCP-only, neither sufficient alone). Opt-in
#   add-ons (default off): rathole ingress TCP 443 + 2333 (rathole_ingress_
#   enabled); coturn STUN UDP+TCP 3478 (coturn_enabled, F-192) and its TURN
#   relay range UDP 49152-65535 (coturn_turn_enabled). No port 80 ACME
#   challenge (certs out-of-band via offline-CA, not Let's Encrypt).
# - DNS publishes per-relay hostname under `goat-relay-<name>` + adds the
#   instance to the `goat-relay` round-robin pool (separate from the
#   legacy `netbird` round-robin during blue-green cutover, per
#   docs/design/relay-blue-green-cutover.md).
# - No rathole credentials secret (goat-relay does not use rathole).
# - Same SSM IAM role + Secrets Manager SSH key pattern as the legacy
#   module so Ansible reaches the host the same way.

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

# Resource descriptions. Every one is an input: a security-group description
# is immutable (changing it replaces the group) and a rule's description is
# part of the rule, so a root whose live goatnet carries other text passes it
# in `descriptions` and plans no change. Keys not passed keep these defaults.
locals {
  descriptions = merge({
    security_group  = "goat-relay (Block 33): TCP+UDP 33073 data plane only"
    ssh             = "SSH (restricted)"
    websocket       = "goat-relay WebSocket data plane"
    quic            = "goat-relay QUIC data plane"
    wg_cp0          = "wg-cp0 silent control-plane WireGuard listener (Block 22)"
    rathole_ingress = "rathole: public Netbird mgmt-API / signal enrollment (TCP 443)"
    rathole_control = "rathole control channel (TCP 2333)"
    coturn_stun_udp = "coturn STUN (UDP 3478)"
    coturn_stun_tcp = "coturn STUN (TCP 3478 fallback)"
    coturn_turn     = "coturn TURN relay media range (UDP 49152-65535)"
    ab_websocket    = "A/B stock-relay experiment: WebSocket data plane (temporary)"
    ab_quic         = "A/B stock-relay experiment: QUIC data plane (temporary, F-021)"
    icmp            = "ICMP ping (latency benchmarks)"
    egress          = "All outbound"
    ssh_key_secret  = "SSH private key for ${local.resource_name}"
    rathole_secret  = "rathole relay credentials for ${local.resource_name} (token + public key)"
  }, var.descriptions)
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
  description = local.descriptions.ssh_key_secret
  tags        = { Service = var.service_name }
}

resource "aws_secretsmanager_secret_version" "ssh_key" {
  secret_id     = aws_secretsmanager_secret.ssh_key.id
  secret_string = tls_private_key.ssh.private_key_pem
}

# -- Rathole credential storage (colocated ingress only) -----------------------
# Empty Secrets Manager container, created only when rathole_ingress_enabled.
# The rathole-server ansible deploy (relay-server-rathole.yml) populates it
# (token + relay public key) at deploy time — same pattern as the legacy relay
# module, so ops/ansible/inventory/generate-relays.sh consumes
# module.<x>.rathole_secret_name unchanged.
resource "aws_secretsmanager_secret" "rathole_credentials" {
  count       = var.rathole_ingress_enabled ? 1 : 0
  name        = "${local.resource_name}-rathole-${random_id.secret_suffix.hex}"
  description = local.descriptions.rathole_secret
  tags        = { Service = var.service_name }
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
  description = local.descriptions.security_group
  vpc_id      = local.vpc_id

  # SSH: no public access by default. Use SSM Session Manager for shell
  # access. If direct SSH is needed, pass ssh_cidr_blocks explicitly.
  dynamic "ingress" {
    for_each = length(var.ssh_cidr_blocks) > 0 ? [1] : []
    content {
      from_port   = 22
      to_port     = 22
      protocol    = "tcp"
      cidr_blocks = var.ssh_cidr_blocks
      description = local.descriptions.ssh
    }
  }

  # F-021 — both TCP and UDP must be open. AWS SG default + Docker -p
  # shorthand each default to TCP-only, neither sufficient alone for
  # the QUIC path. The same lesson applies to systemd-native deploys —
  # the SG is the only firewall layer in that case but still must
  # explicitly open UDP.
  ingress {
    from_port   = 33073
    to_port     = 33073
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = local.descriptions.websocket
  }

  ingress {
    from_port   = 33073
    to_port     = 33073
    protocol    = "udp"
    cidr_blocks = ["0.0.0.0/0"]
    description = local.descriptions.quic
  }

  # Block 22 silent control plane (ADR 0313). Opt-in per terraform-prod
  # for the goat-relay tier to host a wg-cp0 server alongside the Rust
  # netbird-relay binary, giving cross-site Spock + mgmt-API + signal-fed
  # traffic active-active forwarding (closes F-060's single-point at
  # relay-rla). Listener answers nothing without a key-allowlisted peer
  # — fingerprintable as silent UDP.
  dynamic "ingress" {
    for_each = var.wg_cp0_enabled ? [1] : []
    content {
      from_port   = var.wg_cp0_port
      to_port     = var.wg_cp0_port
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.wg_cp0
    }
  }

  # Colocated rathole ingress (opt-in). For goatnets behind CGNAT (no public
  # mgmt-host port-forward, e.g. ZAGROS on Starlink), the rathole-server runs
  # ON the relay hosts so a brand-new off-LAN peer can reach the mgmt-API /
  # signal at :443 before it has a mesh identity. This keeps the relay tier
  # all-Rust (goat-relay for data) while colocating the enrollment-ingress
  # tunnel — no separate legacy relay box. Deploy the server side with
  # ops/ansible/relay-server-rathole.yml against [goat_relay_hosts].
  #   443  — public Netbird mgmt-API / signal enrollment entrypoint.
  #   2333 — rathole control channel (mgmt-host's rathole-client dials in).
  dynamic "ingress" {
    for_each = var.rathole_ingress_enabled ? [1] : []
    content {
      from_port   = 443
      to_port     = 443
      protocol    = "tcp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.rathole_ingress
    }
  }

  dynamic "ingress" {
    for_each = var.rathole_ingress_enabled ? [1] : []
    content {
      from_port   = 2333
      to_port     = 2333
      protocol    = "tcp"
      cidr_blocks = var.rathole_cidr_blocks
      description = local.descriptions.rathole_control
    }
  }

  # Coturn STUN (opt-in, F-192). The all-Rust relay tier dropped coturn, so
  # without this every peer is relay-forced (no STUN → no srflx → no direct
  # P2P). UDP 3478 carries STUN; TCP 3478 is the fallback for UDP-blocked
  # peers. The TURN relay media range stays closed unless coturn_turn_enabled
  # (TURN deferred — rels:// already covers relay-fallback).
  dynamic "ingress" {
    for_each = var.coturn_enabled ? [1] : []
    content {
      from_port   = 3478
      to_port     = 3478
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.coturn_stun_udp
    }
  }

  dynamic "ingress" {
    for_each = var.coturn_enabled ? [1] : []
    content {
      from_port   = 3478
      to_port     = 3478
      protocol    = "tcp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.coturn_stun_tcp
    }
  }

  dynamic "ingress" {
    for_each = (var.coturn_enabled && var.coturn_turn_enabled) ? [1] : []
    content {
      from_port   = 49152
      to_port     = 65535
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.coturn_turn
    }
  }

  # A/B EXPERIMENT PORT — default 0 = no rule, no diff, nothing exposed.
  # Opens a SECOND relay data-plane port on this same host so a stock upstream
  # NetBird Go relay can run beside goat-relay and carry the same traffic for
  # comparison (ops/ansible/stock-relay-ab.yml,
  # docs/operations/relay-ab-stock-vs-rust.md). A relay is addressed as
  # rels://host:port, so the second listener needs no second host, no second
  # DNS name and no second certificate — only a port.
  #
  # TCP *and* UDP, for the same reason 33073 opens both: F-021. The Go relay,
  # like goat-relay, serves WebSocket-over-TLS on TCP and QUIC on UDP at the
  # same address. A TCP-only rule leaves the QUIC arm unreachable and the
  # experiment silently degrades to a WS-vs-WS comparison.
  #
  # This is deliberately a plain number rather than a bool: it is a temporary
  # surface, and an operator reading the plan should see exactly which port is
  # being opened. Set it back to 0 to close the surface when the A/B is done.
  dynamic "ingress" {
    for_each = var.ab_stock_relay_port > 0 ? [1] : []
    content {
      from_port   = var.ab_stock_relay_port
      to_port     = var.ab_stock_relay_port
      protocol    = "tcp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.ab_websocket
    }
  }

  dynamic "ingress" {
    for_each = var.ab_stock_relay_port > 0 ? [1] : []
    content {
      from_port   = var.ab_stock_relay_port
      to_port     = var.ab_stock_relay_port
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = local.descriptions.ab_quic
    }
  }

  ingress {
    from_port   = -1
    to_port     = -1
    protocol    = "icmp"
    cidr_blocks = ["0.0.0.0/0"]
    description = local.descriptions.icmp
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
    description = local.descriptions.egress
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
    Purpose = "goat-relay (Block 33 Rust rewrite) data-plane VPS — c7i.2xlarge per ADR 0712"
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

# -- Cloudflare DNS -------------------------------------------------------------
#
# Per-relay public DNS record `goat-relay-<name>.<zone>` plus optional
# membership in the `goat-relay.<zone>` round-robin pool (separate from
# the legacy `netbird` pool during blue-green cutover).

resource "cloudflare_record" "relay" {
  zone_id = var.cloudflare_zone_id
  name    = local.dns_name
  content = aws_eip.relay.public_ip
  type    = "A"
  proxied = false
}

resource "cloudflare_record" "round_robin" {
  count   = var.round_robin_dns ? 1 : 0
  zone_id = var.cloudflare_zone_id
  name    = var.round_robin_dns_name
  content = aws_eip.relay.public_ip
  type    = "A"
  ttl     = 60
  proxied = false
}
