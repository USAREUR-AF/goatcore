# shellcheck shell=bash
# These assignments are the public data interface read by sourcing scripts.
# shellcheck disable=SC2034
# Relay metadata — single source of truth for bash scripts.
#
# Sourced by: publish-service.sh, check.sh, smoke-test.sh, fresh-peer-probe.sh
#
# The values below are examples. A deployment's own relay tier is the file its
# inventory generator writes to ops/relay-env.d/<name>.sh, assigning the same
# variables; every such file is sourced last, in lexical order, so its values win.

RELAY_NAMES=('fra' 'isr' 'mum')
RELAY_IPS=('203.0.113.2' '203.0.113.20' '203.0.113.30')
RELAY_REGIONS=('eu-central-1' 'il-central-1' 'ap-south-1')
RELAY_INSTANCE_IDS=('i-00000cea114dd2d2f' 'i-00000c028101b56e6' 'i-00000aeee9f4c9fcd')
RELAY_KEY_DIR="/tmp/goat-ssh-keys"
RELAY_HOSTS_PATH="/etc/goat-dns/mesh-hosts"

_relay_env_d="$(dirname "${BASH_SOURCE[0]}")/relay-env.d"
for _relay_env_file in "$_relay_env_d"/*.sh; do
  [ -e "$_relay_env_file" ] || continue
  # shellcheck source=/dev/null
  . "$_relay_env_file"
done
unset _relay_env_d _relay_env_file
