#!/usr/bin/env bash
# laptop-mesh-enrol.sh — the VM founder run's operator machine joins the goatnet's mesh
# as the founder's admin peer, the operator machine `join-node --via mesh` assumes.
# Runs ON the laptop, from the checkout; the box is 10.88.0.2 on the lab LAN.
#
#   laptop-mesh-enrol.sh <goatnet> <zone> <netbird version>
#
# 1. the pinned NetBird client, from its release, checksum-verified (as
#    roles/netbird-client-pin does for a host with no NetBird apt source);
# 2. the goatnet's root for that client alone (mesh-enroll-host.yml's form);
# 3. a single-use, one-hour setup key minted on the box (mgmt-host-bootstrap)
#    with the founder's <GOATNET>-PLATFORM-ADMIN group, handed over in a 0600 file;
# 4. netbird up against the founder's management, and wait for a connection.
set -euo pipefail
NET="$1"; ZONE="$2"; VER="$3"
# 1. The pinned NetBird client, from its release, checksum-verified (as roles/netbird-client-pin does without an apt source).
if [ "$(netbird version 2>/dev/null || true)" != "$VER" ]; then
  tmp=$(mktemp -d); base="https://github.com/netbirdio/netbird/releases/download/v$VER"
  curl -fsSL -o "$tmp/netbird.deb" "$base/netbird_${VER}_linux_amd64.deb"
  curl -fsSL -o "$tmp/sums" "$base/netbird_${VER}_checksums.txt"
  want=$(grep " netbird_${VER}_linux_amd64.deb$" "$tmp/sums" | cut -d' ' -f1); [ -n "$want" ]
  echo "$want  $tmp/netbird.deb" | sha256sum -c - >/dev/null
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$tmp/netbird.deb" >/dev/null
fi
# 2. The goatnet's root for this client alone (mesh-enroll-host.yml's form).
sudo install -d -m 0755 /etc/goat/netbird-trust /etc/systemd/system/netbird.service.d
sudo install -m 0644 "ops/enrollment/ca/$NET/root-cert.pem" /etc/goat/netbird-trust/goatnet-root.pem
printf '[Service]\nEnvironment=SSL_CERT_DIR=/etc/ssl/certs:/etc/goat/netbird-trust\n' | sudo tee /etc/systemd/system/netbird.service.d/20-goat-netbird-trust.conf >/dev/null
sudo systemctl daemon-reload; sudo systemctl restart netbird
# 3. A single-use, one-hour key minted on the box, with the founder's PLATFORM-ADMIN group.
UP=$(printf '%s' "$NET" | tr '[:lower:]-' '[:upper:]_')
key=$(ssh -o BatchMode=yes ubuntu@10.88.0.2 "set -e; q() { sudo docker exec postgres-a psql -U postgres -d netbird -tAc \"\$1\"; }; acct=\$(q 'select id from accounts limit 1'); grp=\$(q \"select id from groups where name='${UP}-PLATFORM-ADMIN'\"); [ -n \"\$acct\" ] && [ -n \"\$grp\" ]; sudo mgmt-host-bootstrap create-setup-key --account \"\$acct\" --role bootstrap --ttl 1h --usage 1 --name lab-laptop-\$(date -u +%Y%m%dT%H%M%SZ) --auto-groups \"\$grp\"" | sed -n 's/.*plaintext:[[:space:]]*\([^[:space:]]*\).*/\1/p')
[ -n "$key" ]
f=$(mktemp); chmod 600 "$f"; printf '%s' "$key" > "$f"; unset key
# 4. Up, against the founder's management.
sudo netbird up --management-url "https://netbird.$ZONE:443" --setup-key-file "$f" >/dev/null; rm -f "$f"
for _ in $(seq 1 30); do sudo netbird status 2>/dev/null | grep -q '^Management: Connected' && break; sleep 2; done
sudo netbird status | grep -E '^(Management|Signal|NetBird IP|Peers count)'
