# host-node module — one node of a four-node host site (ADR 1141), realized on
# EC2 instead of on-prem hardware.
#
# WHAT THIS IS. ADR 1141 fixed the minimum host-site shape at four nodes: a core
# pair (mgmt control plane, active + warm standby) and an obs pair (observability
# and storage, active + cutover-gated standby). It also said, in as many words,
# that "core" and "obs" name ROLES, not hardware. This module is the first
# instance of that sentence being load-bearing: the same roles, provisioned on
# rented compute.
#
# WHY CLOUD, WHICH IS AGAINST THE GRAIN. The substrate's whole posture is
# "use the cloud to find itself, the tactical edge to run itself" — a mgmt
# control plane living in someone else's datacenter is a deliberate deviation
# from that, and it is declared as one in docs/design/deliberate-deltas.md and
# reasoned in the cloud host site design doc. The short version: a single site
# LAN carrying the control planes of several goatnets is one failure domain, and
# rented compute in a second failure domain is what is actually available when
# there is no second on-prem site. It is a procurement fact, not
# an architectural preference.
#
# WHAT IT INHERITS. The proven cloud-host pattern from mesh-dns-host (ADR 1056)
# and relay-health-host (ADR 1061): Ubuntu 24.04, default VPC, SSM-managed with
# port 22 closed, an S3 grant for the ansible aws_ssm file channel, mesh
# enrollment from cloud-init, Docker with a host-wide log bound, and
# lifecycle ignore_changes on [ami, user_data] so a routine apply can never
# destroy a long-lived enrolled host.
#
# WHAT IT ADDS.
#   - A separate encrypted EBS data volume mounted at /opt/docker-data — the
#     SAME mountpoint the on-prem LUKS volume uses (ops/ansible/mgmt-host-luks.yml),
#     so docker's data-root and every service state dir land where every
#     existing role already expects them. No role needs a cloud special-case.
#   - Persistent enrollment retry. The outage is the case that
#     matters: these nodes are being built WHILE the goatnet's management plane
#     is down, so a five-shot retry in cloud-init would give up and leave a host
#     that is provisioned but not a peer. A systemd timer keeps trying until it
#     lands, and disables itself once it has.
#   - AZ separation as a required input, not an option — see var.az.
#
# WHAT IT DELIBERATELY DOES NOT DO. No LUKS. Volume encryption here is EBS/KMS
# at the hypervisor, which is a different threat model from ADR 0612's
# passphrase/TPM-unlocked local disk: it defends a decommissioned physical disk,
# not a seized running host, and the key is held by the cloud provider. Declared
# in deliberate-deltas.md rather than papered over.

terraform {
  required_providers {
    aws = { source = "hashicorp/aws" }
  }
}

locals {
  hostname = "mgmt-${var.site}-${var.role}-${var.index}"

  # Explicit id wins; else the newest self-owned bake for this role; else stock
  # Ubuntu. try() keeps the fallback working when no bake exists yet, which is
  # the day-one state and must not be a plan-time error.
  prebaked_ami = var.ami_id != "" ? var.ami_id : try(data.aws_ami.goat_host[0].id, "")
  resolved_ami = local.prebaked_ami != "" ? local.prebaked_ami : data.aws_ami.ubuntu.id
  image_path   = local.prebaked_ami != "" ? "prebaked" : "stock-ubuntu-slow-path"
}

# ---------------------------------------------------------------------------
# Machine image. TWO candidates, and which one is used decides whether a
# bringup takes four minutes or forty.
#
# PREFERRED: a pre-baked goat host AMI (ops/bootstrap/build-image-ec2.sh),
# tagged GoatImageClass=<role>. It already carries docker, containerd, netbird,
# chrony AND the pinned container images, so first boot installs nothing and
# pulls nothing. This is the artifact that makes the Block 68 one-hour budget
# reachable on cloud hardware.
#
# FALLBACK: stock Ubuntu 24.04. Works, but first boot then apt-installs the
# engine and every container image comes over the wire — the slow path. It
# exists so a site can still be built when no bake has been published yet
# (which is exactly the state on day one), never as the steady state.
#
# The bootstrap script below is IDENTICAL either way and every step is
# skip-if-present, so there is one path with a fast case, not two paths
# (doctrine P1). The chosen AMI is reported in the module output so nobody has
# to guess which case they got.
# ---------------------------------------------------------------------------
data "aws_ami" "goat_host" {
  count       = var.ami_id == "" ? 1 : 0
  most_recent = true
  owners      = ["self"]

  filter {
    name   = "tag:GoatImageClass"
    values = [var.role]
  }
  filter {
    name   = "state"
    values = ["available"]
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

# ---------------------------------------------------------------------------
# IAM — SSM managed core (shell + ansible with port 22 closed) plus scoped
# access to the goatnet's SSM staging bucket. Same shape as relay-health-host;
# the managed-core policy does not grant the S3 the aws_ssm connection plugin
# needs, so it is attached explicitly.
# ---------------------------------------------------------------------------
resource "aws_iam_role" "this" {
  name = local.hostname
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
  name = local.hostname
  role = aws_iam_role.this.name
}

# ---------------------------------------------------------------------------
# Security group. Egress-only by default. Public ingress is rendered ONLY when
# public_enabled — the same gated-flip shape as relay-health-host, for the same
# reason: a public port that appears as a side effect of a routine apply is how
# the no-public-exposure invariant dies quietly.
# ---------------------------------------------------------------------------
resource "aws_security_group" "this" {
  name        = local.hostname
  description = "${var.goatnet} four-node host site: ${var.role}-${var.index} at ${var.site}. Egress-only unless public_enabled (the deliberate enrollment-apex flip)."
  vpc_id      = data.aws_vpc.default.id

  dynamic "ingress" {
    for_each = var.public_enabled ? var.public_ingress_ports : []
    content {
      from_port   = ingress.value
      to_port     = ingress.value
      protocol    = "tcp"
      cidr_blocks = var.public_ingress_cidrs
      description = "public tcp/${ingress.value} (enrollment-apex flip)"
    }
  }

  # WireGuard data plane. Opened with the same flip: a cloud-resident host with
  # a reachable UDP port lets peers form DIRECT tunnels to it. On-prem, Starlink
  # CGNAT makes that impossible and every peer pays the relay hop; here it is
  # simply available, and it is the one genuine capability the cloud site has
  # that an on-prem host does not.
  dynamic "ingress" {
    for_each = var.public_enabled ? [var.wireguard_port] : []
    content {
      from_port   = ingress.value
      to_port     = ingress.value
      protocol    = "udp"
      cidr_blocks = ["0.0.0.0/0"]
      description = "wireguard/netbird data plane udp/${ingress.value}"
    }
  }

  egress {
    description = "Allow outbound IPv4 traffic"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = merge(var.tags, { Name = local.hostname })
}

# ---------------------------------------------------------------------------
# Data volume. Created here so one module call == one whole node, and so the
# volume's lifecycle is independent of the instance: an instance can be
# -replace'd (OS bump, bootstrap change) without touching the state that lives
# on it. prevent_destroy is deliberate — this volume holds Postgres, the audit
# store and MinIO's data, and `terraform destroy` should have to fight for it.
# ---------------------------------------------------------------------------
resource "aws_ebs_volume" "data" {
  availability_zone = var.az
  size              = var.data_volume_size
  type              = "gp3"
  iops              = var.data_volume_iops
  throughput        = var.data_volume_throughput
  encrypted         = true

  lifecycle {
    prevent_destroy = true
  }

  tags = merge(var.tags, { Name = "${local.hostname}-data" })
}

# ---------------------------------------------------------------------------
# Bootstrap. Deliberately thin: docker, a log bound, the data volume, and the
# mesh client. EVERYTHING ELSE IS ANSIBLE'S JOB, reached over SSM. A cloud-init
# script that installs services is a second deployment path, and doctrine P1
# says there is only one.
# ---------------------------------------------------------------------------
locals {
  # The data volume's device. Nitro instances rename /dev/sdf to a /dev/nvme*
  # path that is not predictable from the attachment, so the bootstrap resolves
  # it by EBS volume-id through the nvme-cli serial instead of guessing.
  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    exec > >(tee /var/log/goat-host-node-bootstrap.log) 2>&1
    echo "[$(date -Is)] ${local.hostname} bootstrap starting"

    export DEBIAN_FRONTEND=noninteractive
    if ! command -v jq >/dev/null 2>&1 || ! command -v nvme >/dev/null 2>&1; then
      apt-get update -qq
      apt-get install -y -qq ca-certificates curl gnupg jq nvme-cli
    fi
    echo "[$(date -Is)] image path: ${local.image_path}"

    # ---- Data volume ------------------------------------------------------
    # Mounted at /opt/docker-data to match the on-prem LUKS mountpoint exactly
    # (ops/ansible/mgmt-host-luks.yml). Every role downstream — docker-engine's
    # data-root assert included — then works unmodified.
    #
    # Device resolution: on Nitro the attachment name is advisory. Ask nvme-cli
    # for the volume whose serial matches, and fail loudly rather than format
    # the wrong disk. Formatting is guarded on blkid finding NO existing
    # filesystem, so a reboot or a re-run never wipes live data.
    DATA_DEV=""
    for _ in $(seq 1 30); do
      for dev in /dev/nvme*n1; do
        [ -b "$dev" ] || continue
        serial=$(nvme id-ctrl -o json "$dev" 2>/dev/null | jq -r '.sn // empty' | tr -d ' ')
        # EBS reports the volume id as the serial, without the '-'
        case "$serial" in
          vol*) vid="vol-$${serial#vol}" ;;
          *) continue ;;
        esac
        if [ "$vid" = "${aws_ebs_volume.data.id}" ]; then DATA_DEV="$dev"; break; fi
      done
      [ -n "$DATA_DEV" ] && break
      echo "[$(date -Is)] waiting for data volume ${aws_ebs_volume.data.id} to attach"; sleep 5
    done

    if [ -z "$DATA_DEV" ]; then
      echo "[$(date -Is)] FATAL: data volume ${aws_ebs_volume.data.id} never appeared. NOT formatting anything."
      exit 1
    fi
    echo "[$(date -Is)] data volume ${aws_ebs_volume.data.id} is $DATA_DEV"

    if ! blkid "$DATA_DEV" >/dev/null 2>&1; then
      echo "[$(date -Is)] no filesystem on $DATA_DEV — creating ext4"
      mkfs.ext4 -L goat-data "$DATA_DEV"
    else
      echo "[$(date -Is)] $DATA_DEV already carries a filesystem — leaving it alone"
    fi

    mkdir -p /opt/docker-data
    grep -q '^LABEL=goat-data' /etc/fstab || \
      echo 'LABEL=goat-data /opt/docker-data ext4 defaults,nofail 0 2' >> /etc/fstab
    mount -a
    findmnt /opt/docker-data || { echo "[$(date -Is)] FATAL: /opt/docker-data did not mount"; exit 1; }

    # ---- Docker -----------------------------------------------------------
    # SKIP-IF-PRESENT. On a pre-baked goat AMI the engine is already here and
    # this whole block is a no-op costing about a second; on stock Ubuntu it is
    # the slow path and says so in the log. One script, two speeds — not two
    # scripts (doctrine P1).
    if command -v docker >/dev/null 2>&1; then
      echo "[$(date -Is)] docker already present (pre-baked image) — skipping engine install"
    else
      echo "[$(date -Is)] docker ABSENT — stock-image slow path, installing from download.docker.com"
      install -m 0755 -d /etc/apt/keyrings
      curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
      chmod a+r /etc/apt/keyrings/docker.gpg
      echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo $VERSION_CODENAME) stable" \
        > /etc/apt/sources.list.d/docker.list
      apt-get update -qq
      apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-compose-plugin
    fi

    # data-root on the data volume (F-093: docker AND containerd off the root
    # disk) + the host-wide log bound (F-229: bound EVERY container's log, not
    # per-compose-file, so a service that forgets still cannot fill the disk).
    install -d -m 0755 /etc/docker
    cat > /etc/docker/daemon.json <<'JSON'
    {
      "data-root": "/opt/docker-data/docker",
      "log-driver": "json-file",
      "log-opts": { "max-size": "50m", "max-file": "5" }
    }
    JSON
    install -d -m 0755 /etc/containerd
    mkdir -p /opt/docker-data/containerd
    [ -f /etc/containerd/config.toml ] || containerd config default > /etc/containerd/config.toml
    sed -i 's#^root = .*#root = "/opt/docker-data/containerd"#' /etc/containerd/config.toml
    systemctl restart containerd
    systemctl enable --now docker

    # ---- Mesh client (skip-if-present, same reasoning as docker) ----------
    if command -v netbird >/dev/null 2>&1; then
      echo "[$(date -Is)] netbird already present (pre-baked image) — skipping client install"
    else
      echo "[$(date -Is)] netbird ABSENT — stock-image slow path, installing from pkgs.netbird.io"
      curl -fsSL https://pkgs.netbird.io/debian/public.key | gpg --dearmor -o /usr/share/keyrings/netbird-archive-keyring.gpg
      echo "deb [signed-by=/usr/share/keyrings/netbird-archive-keyring.gpg] https://pkgs.netbird.io/debian stable main" \
        > /etc/apt/sources.list.d/netbird.list
      apt-get update -qq
      apt-get install -y -qq netbird
    fi

    %{if var.netbird_enroll_enabled~}
    # PERSISTENT enrollment retry, not a boot-time best-effort loop.
    #
    # This module's first use is standing up a site WHILE the goatnet's
    # management plane is down — that is the reason it exists. A cloud-init
    # loop that tries five times and gives up would leave a provisioned host
    # that is not a peer, and nothing would ever notice. The timer retries
    # every two minutes and removes itself the moment enrollment lands, so it
    # is self-limiting rather than a permanent background process.
    cat > /usr/local/sbin/goat-mesh-enroll <<'ENROLL'
    #!/bin/bash
    set -uo pipefail
    if netbird status 2>/dev/null | grep -qi 'Management: Connected'; then
      echo "already enrolled and connected; disabling retry timer"
      systemctl disable --now goat-mesh-enroll.timer || true
      exit 0
    fi
    netbird up --setup-key "$(cat /etc/goat-netbird-setup-key)" \
               --management-url "$(cat /etc/goat-netbird-mgmt-url)" \
               --hostname "$(cat /etc/goat-netbird-hostname)"
    ENROLL
    chmod 0700 /usr/local/sbin/goat-mesh-enroll

    # The setup key is enrollment-time-only material and rotates on its own
    # cadence; it is written 0600 and is irrelevant once wt0 is up.
    umask 077
    printf '%s' '${var.netbird_setup_key}' > /etc/goat-netbird-setup-key
    printf '%s' '${var.netbird_mgmt_url}'  > /etc/goat-netbird-mgmt-url
    printf '%s' '${local.hostname}'        > /etc/goat-netbird-hostname
    umask 022

    cat > /etc/systemd/system/goat-mesh-enroll.service <<'UNIT'
    [Unit]
    Description=Enroll this host on the goat mesh (retries until it lands)
    After=network-online.target netbird.service
    Wants=network-online.target
    [Service]
    Type=oneshot
    ExecStart=/usr/local/sbin/goat-mesh-enroll
    UNIT

    cat > /etc/systemd/system/goat-mesh-enroll.timer <<'UNIT'
    [Unit]
    Description=Retry goat mesh enrollment until it succeeds
    [Timer]
    OnBootSec=30s
    OnUnitActiveSec=2min
    [Install]
    WantedBy=timers.target
    UNIT

    systemctl daemon-reload
    systemctl enable --now goat-mesh-enroll.timer
    %{else~}
    echo "[$(date -Is)] mesh enrollment DISABLED for this node by netbird_enroll_enabled=false"
    %{endif~}

    echo "[$(date -Is)] bootstrap complete — services are ansible's job, over SSM."
  EOT
}

resource "aws_instance" "this" {
  ami                    = local.resolved_ami
  instance_type          = var.instance_type
  availability_zone      = var.az
  iam_instance_profile   = aws_iam_instance_profile.this.name
  vpc_security_group_ids = concat([aws_security_group.this.id], var.extra_security_group_ids)
  user_data              = local.user_data

  # NOT user_data_replace_on_change. relay-health-host sets it and then has to
  # neutralise it with ignore_changes; the honest form is to not arm the gun.
  # A bootstrap change wants a deliberate -replace, never a side effect.

  metadata_options {
    http_endpoint               = "enabled"
    http_tokens                 = "required" # IMDSv2 only
    http_put_response_hop_limit = 2          # containers may reach IMDS
  }

  root_block_device {
    volume_size = var.root_volume_size
    volume_type = "gp3"
    encrypted   = true
  }

  # Pin BOTH the AMI and the rendered user_data against incidental drift. These
  # are long-lived stateful hosts:
  #   - ami: the most_recent lookup moves every time Canonical publishes, so
  #     without this ANY apply touching the module destroys and recreates a
  #     live control-plane node.
  #   - user_data: carries the netbird setup key, which rotates on its own
  #     cadence. Without this, the first apply after a key rotation shows a
  #     diff on a host whose enrollment has long since stopped caring.
  lifecycle {
    ignore_changes = [ami, user_data]
  }

  tags = merge(var.tags, {
    Name      = local.hostname
    GoatSite  = var.site
    GoatRole  = var.role
    GoatIndex = var.index
  })
}

resource "aws_volume_attachment" "data" {
  device_name = "/dev/sdf"
  volume_id   = aws_ebs_volume.data.id
  instance_id = aws_instance.this.id
}

# Elastic IP — only when public_enabled. A stable address is what the failover
# DNS record points at, so it must survive an instance stop/start.
resource "aws_eip" "this" {
  count    = var.public_enabled ? 1 : 0
  instance = aws_instance.this.id
  domain   = "vpc"
  tags     = merge(var.tags, { Name = local.hostname })
}
