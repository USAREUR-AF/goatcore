# shellcheck shell=bash
# Each goatnet's relay TLS posture, read by generate-relays.sh: le-dns01 (publicly trusted,
# Cloudflare DNS-01) or self-signed. A goatnet with no row is refused. goatlite adds its row here.
relay_tls_posture_for() {
  case "$1" in
    *) return 1 ;;
  esac
}
