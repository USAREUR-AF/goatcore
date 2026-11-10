#!/usr/bin/env bash
# mint-site-certs.sh — standup phase 4: mint every leaf cert from a
# manifest, in one loop.
#
# (docs/design/goatnet-turnkey-standup.md §"The three automations", item 2.)
# Before this script, phase 5 was one hand-run `cert-sign` per service. The
# per-service subjects/SANs/EKUs are
# deterministic data, so they live in a committed manifest and this script is
# just the loop.
#
# Usage:
#   ops/scripts/mint-site-certs.sh <site-id> [--manifest <yaml>] [--dry-run] [--only <name>]
#                                            [--mint-ca-if-missing | --mint-ca-only]
#
#   <site-id>   lowercase instance id (e.g. site-a). Selects the default
#               manifest ops/enrollment/manifests/<site-id>-certs.yaml.
#   --manifest  explicit manifest path (overrides the default).
#   --dry-run   print each cert-sign invocation; mint nothing.
#   --only      mint a single named leaf (rotation / repair path).
#   --mint-ca-if-missing
#               goatlite (ADR 1213 D4, docs/design/goatlite.md §5 row 5): when the
#               manifest's ca.root_cert AND ca.root_key both do not exist, mint
#               the goatnet's CA first — an age identity at ca.age_identity
#               (age-keygen, 0600, if absent) as the SINGLE recipient, then
#               `ca-init --mode dev --allow-single-recipient` with the manifest's
#               ca_id, the root key at ca.root_key, the root cert at
#               ca.root_cert and the public key under ops/enrollment/public-keys/.
#               One of the pair present without the other is refused (half a
#               CA is never overwritten). Without the flag a missing input still
#               dies, as before.
#   --mint-ca-only
#               the same CA step and nothing else: exit after it, mint no leaf.
#               goatlite-render.py calls this so the CA exists before the pins
#               row renders (the goatnet's intent is signed with that key).
#
# IDEMPOTENT: a leaf whose out-cert already exists is skipped — re-running
# against a half-minted site fills only the gaps. Rotation of an existing
# leaf = delete the pair (or use --only after deleting), re-run.
#
# The manifest carries NO secrets: subjects, SANs, EKUs and output paths only.
# Keys never leave the operator machine; out-dir defaults to
# <state dir>/<site-id>-leaves, where the state dir is what the project declares
# (deploy.state_dir in ops/goatbuilder/goatbuilder.yaml), else ~/.goat/state.
#
# Class-A warning (instance-variability-model.md §1): URI SANs are
# freeze-critical identity. A new site's manifest must use ITS OWN
# `urn:goat:<site>:...` prefix — copying another site's manifest without
# re-threading the uri_sans mints certs that claim the wrong goatnet.
set -euo pipefail

die() { echo "mint-site-certs: $*" >&2; exit 1; }

command -v yq >/dev/null || die "yq is required (same dependency as ops/scripts/rollback.sh)"

SITE="${1:-}"; [ -n "$SITE" ] || die "usage: mint-site-certs.sh <site-id> [--manifest <yaml>] [--dry-run] [--only <name>]"
shift

# Resolve repo root from the script's own location so worktree invocations
# read their own manifest while state stays in $HOME (newsite-secrets.sh
# pattern).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

MANIFEST="$REPO_ROOT/ops/enrollment/manifests/${SITE}-certs.yaml"
DRY_RUN=0
ONLY=""
MINT_CA=0
MINT_CA_ONLY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --manifest) MANIFEST="$2"; shift 2 ;;
    --dry-run)  DRY_RUN=1; shift ;;
    --only)     ONLY="$2"; shift 2 ;;
    --mint-ca-if-missing) MINT_CA=1; shift ;;
    --mint-ca-only) MINT_CA=1; MINT_CA_ONLY=1; shift ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [ ! -f "$MANIFEST" ]; then
  # Point at the template rather than just refusing: previously the only
  # Point at the template rather than just refusing: copying another goatnet's
  # manifest mints certs claiming that goatnet.
  die "manifest not found: $MANIFEST

  This goatnet has no cert manifest yet. Start from the template, which carries
  the shape with every site-specific value marked:

    cp ops/enrollment/manifests/TEMPLATE-certs.yaml \\
       ops/enrollment/manifests/${SITE}-certs.yaml
    \$EDITOR ops/enrollment/manifests/${SITE}-certs.yaml   # replace <SITE> + TBD_
    ops/scripts/mint-site-certs.sh ${SITE} --dry-run

  Do NOT copy another goatnet's manifest: its uri_sans name that goatnet, and a
  URI SAN is freeze-critical identity (Class A, instance-variability-model.md §1)."
fi

msite="$(yq -r '.site_id' "$MANIFEST")"
[ "$msite" = "$SITE" ] || die "manifest site_id '$msite' != requested site '$SITE' (wrong-goatnet guard)"

# shellcheck disable=SC2088  # quoted tilde is deliberate: manifests write literal "~/" paths
expand_home() { case "$1" in "~/"*) printf '%s/%s' "$HOME" "${1#\~/}" ;; *) printf '%s' "$1" ;; esac; }

ROOT_CERT="$(expand_home "$(yq -r '.ca.root_cert' "$MANIFEST")")"
case "$ROOT_CERT" in /*) ;; *) ROOT_CERT="$REPO_ROOT/$ROOT_CERT" ;; esac
ROOT_KEY="$(expand_home "$(yq -r '.ca.root_key' "$MANIFEST")")"
AGE_IDENTITY="$(expand_home "$(yq -r '.ca.age_identity // "~/.config/age/keys.txt"' "$MANIFEST")")"
CA_ID="$(yq -r '.ca.ca_id' "$MANIFEST")"
# Where this machine keeps a project's local state when the manifest names no
# out_dir: what the project declares (deploy.state_dir in
# ops/goatbuilder/goatbuilder.yaml, the key goatbuilder reads), else ~/.goat/state.
STATE_DIR="$(yq -r '.deploy.state_dir // ""' "$REPO_ROOT/ops/goatbuilder/goatbuilder.yaml" 2>/dev/null || true)"
# shellcheck disable=SC2088  # quoted tilde is deliberate: expand_home expands it
[ -n "$STATE_DIR" ] || STATE_DIR="~/.goat/state"
OUT_DIR="$(expand_home "$(yq -r ".out_dir // \"${STATE_DIR}/${SITE}-leaves\"" "$MANIFEST")")"
# The audit log cert-sign appends to: ca-audit.jsonl beside the state
# directory, named on every call so the file is the project's and not a default
# built into the tool (the path ops/scripts/project-setting.sh ca-audit-log prints).
case "${STATE_DIR%/}" in */*) AUDIT_LOG="${STATE_DIR%/}"; AUDIT_LOG="$(expand_home "${AUDIT_LOG%/*}/ca-audit.jsonl")" ;; *) AUDIT_LOG=./ca-audit.jsonl ;; esac
# The Subject O of every certificate minted here: what the checkout declares
# as goat_cert_organization in ops/ansible/group_vars/all.yml. With none
# declared the tools issue their own default.
CERT_ORG="$(yq -r '.goat_cert_organization // ""' "$REPO_ROOT/ops/ansible/group_vars/all.yml" 2>/dev/null || true)"
# takes_org <tool>: the tool has --org. A binary on PATH built before the flag
# existed issues the organisation compiled into it, so the flag is left off
# and the run says so once.
takes_org() { local help; help="$("$1" --help 2>&1 || true)"; case "$help" in *"-org "*) return 0 ;; esac; return 1; }
org_args() {  # org_args <tool> -> prints "--org" and the organisation, one per line, when both apply
  [ -n "$CERT_ORG" ] || return 0
  if takes_org "$1"; then printf '%s\n%s\n' --org "$CERT_ORG"; return 0; fi
  echo "mint-site-certs: $1 has no --org (built before the flag): it issues the organisation compiled into it. Rebuild it from ops/enrollment to state '$CERT_ORG' on the call." >&2
}

# Resolve post-enrol mesh-IP SAN placeholders from the site's group_vars so the
# router cert always carries the node's real wt0 IP without a hand-edit of the
# manifest. The manifest keeps the TBD_..._MESH_IP marker to DOCUMENT the
# post-enrol dependency; this fills it from the authoritative source (mesh_ip_a /
# mesh_ip_b) and FAILS CLOSED on any placeholder it cannot resolve — rather than
# minting a cert with a literal "TBD_..." SAN, which passes minting and only
# fails later as a zenoh-mTLS SAN mismatch against the real mesh IP.
GROUP_VARS="$REPO_ROOT/ops/ansible/group_vars/${SITE//-/_}_hosts/vars.yml"
MESH_IP_A="$(yq -r '.mesh_ip_a // ""' "$GROUP_VARS" 2>/dev/null || true)"
MESH_IP_B="$(yq -r '.mesh_ip_b // ""' "$GROUP_VARS" 2>/dev/null || true)"
resolve_san() {
  local v="$1"
  case "$v" in
    TBD_*_CORE_A_MESH_IP|TBD_*_MESH_IP_A|TBD_ROUTER_MESH_IP) v="$MESH_IP_A" ;;
    TBD_*_CORE_B_MESH_IP|TBD_*_MESH_IP_B) v="$MESH_IP_B" ;;
  esac
  case "$v" in
    ""|TBD_*) die "unresolved mesh-IP SAN placeholder '$1' for site '$SITE'. Fill mesh_ip_a / mesh_ip_b (the post-enrol wt0 IPs) in $GROUP_VARS before minting — the router cert must carry the real mesh IP or zenoh mTLS fails SAN validation." ;;
  esac
  printf '%s' "$v"
}

CERT_SIGN="${CERT_SIGN:-cert-sign}"
if ! command -v "$CERT_SIGN" >/dev/null; then
  # Build-if-missing, like the site-bootstrap NTP pre-task.
  if command -v go >/dev/null && [ -d "$REPO_ROOT/ops/enrollment/cmd/cert-sign" ]; then
    echo "cert-sign not on PATH — building from ops/enrollment/cmd/cert-sign"
    ( cd "$REPO_ROOT/ops/enrollment" && go build -o "$REPO_ROOT/ops/enrollment/bin/cert-sign" ./cmd/cert-sign )
    CERT_SIGN="$REPO_ROOT/ops/enrollment/bin/cert-sign"
  else
    die "cert-sign not found and cannot build (need go + ops/enrollment/cmd/cert-sign)"
  fi
fi

# --mint-ca-if-missing: the goatnet's own CA, minted here when neither half exists.
# Build-if-missing for ca-init follows the cert-sign pattern above. The age
# identity is the operator's ~/.config/age/keys.txt (the manifest's ca.age_identity)
# — the same identity cert-sign decrypts the key with on every later mint.
build_tool() {  # build_tool <name> -> prints the binary path, builds from ops/enrollment/cmd/<name> if absent
  local name="$1" bin
  bin="$(command -v "$name" 2>/dev/null || true)"
  if [ -z "$bin" ]; then
    if command -v go >/dev/null && [ -d "$REPO_ROOT/ops/enrollment/cmd/$name" ]; then
      echo "$name not on PATH — building from ops/enrollment/cmd/$name" >&2
      ( cd "$REPO_ROOT/ops/enrollment" && go build -o "$REPO_ROOT/ops/enrollment/bin/$name" "./cmd/$name" ) >&2
      bin="$REPO_ROOT/ops/enrollment/bin/$name"
    else
      die "$name not found and cannot build (need go + ops/enrollment/cmd/$name)"
    fi
  fi
  printf '%s' "$bin"
}

if [ "$MINT_CA" = 1 ] && [ "$DRY_RUN" = 0 ]; then
  if [ -f "$ROOT_CERT" ] && [ -f "$ROOT_KEY" ]; then
    echo "mint-site-certs: CA present ($ROOT_CERT + $ROOT_KEY) — nothing to mint"
  elif [ -f "$ROOT_CERT" ] || [ -f "$ROOT_KEY" ]; then
    die "--mint-ca-if-missing: half a CA exists (cert: $ROOT_CERT, key: $ROOT_KEY) — refusing to overwrite either half; restore the missing half or move both aside"
  else
    case "$CA_ID" in ""|null|TBD_*) die "--mint-ca-if-missing: the manifest's ca.ca_id is '$CA_ID'; set it (e.g. dev-${SITE}-ca-ecdsa-$(date -u +%F)) before minting" ;; esac
    command -v age-keygen >/dev/null || die "--mint-ca-if-missing: age-keygen is required (brew install age / apt install age)"
    if [ ! -f "$AGE_IDENTITY" ]; then
      mkdir -p "$(dirname "$AGE_IDENTITY")"
      ( umask 077 && age-keygen -o "$AGE_IDENTITY" ) >/dev/null 2>&1 || die "age-keygen failed for $AGE_IDENTITY"
      chmod 0600 "$AGE_IDENTITY"
      echo "mint-site-certs: generated age identity $AGE_IDENTITY (mode 0600) — it decrypts this goatnet's CA key; back it up"
    fi
    RECIPIENT="$(age-keygen -y "$AGE_IDENTITY")"
    [ -n "$RECIPIENT" ] || die "--mint-ca-if-missing: no public key in $AGE_IDENTITY"
    CA_INIT="$(build_tool ca-init)"
    mkdir -p "$(dirname "$ROOT_CERT")" "$(dirname "$ROOT_KEY")" "$REPO_ROOT/ops/enrollment/public-keys"
    echo "mint-site-certs: minting the $SITE CA ($CA_ID) — single age recipient $RECIPIENT (ADR 1213 D4)"
    ca_org=(); while IFS= read -r a; do ca_org+=( "$a" ); done < <(org_args "$CA_INIT")
    "$CA_INIT" --mode dev --allow-single-recipient --recipients "$RECIPIENT" --id "$CA_ID" \
      --out "$ROOT_KEY" --gen-root-cert "$ROOT_CERT" --age-identity "$AGE_IDENTITY" \
      --root-cn "$SITE goatlite CA" --public-key-dir "$REPO_ROOT/ops/enrollment/public-keys" \
      ${ca_org[@]+"${ca_org[@]}"} >/dev/null
    [ -f "$ROOT_CERT" ] && [ -f "$ROOT_KEY" ] || die "ca-init returned but $ROOT_CERT / $ROOT_KEY are not both present"
    echo "mint-site-certs: CA minted — key $ROOT_KEY (age), root cert $ROOT_CERT, public key ops/enrollment/public-keys/$CA_ID.pem (commit the cert + public key; never the key)"
  fi
fi

if [ "$MINT_CA_ONLY" = 1 ]; then
  [ -f "$ROOT_CERT" ] && [ -f "$ROOT_KEY" ] || die "--mint-ca-only: CA still incomplete ($ROOT_CERT, $ROOT_KEY)"
  exit 0
fi

for f in "$ROOT_CERT" "$ROOT_KEY" "$AGE_IDENTITY"; do
  [ "$DRY_RUN" = 1 ] || [ -f "$f" ] || die "missing input: $f"
done
[ "$DRY_RUN" = 1 ] || mkdir -p "$OUT_DIR"

# Resolved once: the same flags go on every leaf.
leaf_org=(); while IFS= read -r a; do leaf_org+=( "$a" ); done < <(org_args "$CERT_SIGN")

count="$(yq -r '.leaves | length' "$MANIFEST")"
[ "$count" -gt 0 ] || die "manifest has no leaves"

minted=0 skipped=0
i=0
while [ "$i" -lt "$count" ]; do
  name="$(yq -r ".leaves[$i].name" "$MANIFEST")"
  if [ -n "$ONLY" ] && [ "$name" != "$ONLY" ]; then i=$((i+1)); continue; fi
  cn="$(yq -r ".leaves[$i].cn" "$MANIFEST")"
  eku="$(yq -r ".leaves[$i].eku // \"clientAuth\"" "$MANIFEST")"
  out_cert="$OUT_DIR/${name}-cert.pem"
  out_key="$OUT_DIR/${name}-key.pem"

  if [ -f "$out_cert" ] && [ "$DRY_RUN" = 0 ]; then
    echo "  = $name (exists, skipped)"
    skipped=$((skipped+1)); i=$((i+1)); continue
  fi
  # Orphan-key guard: cert-sign does NOT overwrite an existing 0400 key, so
  # minting a new cert over a leftover key yields a cert/key KeyMismatch that
  # surfaces only as a service crash-loop.
  # Rotation = delete BOTH; a lone key here is always stale.
  if [ -f "$out_key" ] && [ "$DRY_RUN" = 0 ]; then
    echo "  ! $name: removing orphan key $out_key (cert absent — stale pair half)"
    rm -f "$out_key"
  fi

  args=( --root-cert "$ROOT_CERT" --root-key "$ROOT_KEY" --age-identity "$AGE_IDENTITY"
         --ca-id "$CA_ID" --site-id "$SITE" --cn "$cn" --eku "$eku"
         --out-cert "$out_cert" --out-key "$out_key" --audit-log "$AUDIT_LOG"
         ${leaf_org[@]+"${leaf_org[@]}"} )
  n_san="$(yq -r ".leaves[$i].sans | length" "$MANIFEST")"
  j=0; while [ "$j" -lt "$n_san" ]; do args+=( --san "$(resolve_san "$(yq -r ".leaves[$i].sans[$j]" "$MANIFEST")")" ); j=$((j+1)); done
  n_uri="$(yq -r ".leaves[$i].uri_sans // [] | length" "$MANIFEST")"
  j=0; while [ "$j" -lt "$n_uri" ]; do args+=( --uri-san "$(yq -r ".leaves[$i].uri_sans[$j]" "$MANIFEST")" ); j=$((j+1)); done

  if [ "$DRY_RUN" = 1 ]; then
    printf 'DRY: %s' "$CERT_SIGN"; printf ' %q' "${args[@]}"; printf '\n'
  else
    echo "  + $name"
    "$CERT_SIGN" "${args[@]}"
    if [ "$(yq -r ".leaves[$i].append_root // false" "$MANIFEST")" = "true" ]; then
      cat "$out_cert" "$ROOT_CERT" > "$out_cert.tmp" && mv "$out_cert.tmp" "$out_cert"
    fi
    minted=$((minted+1))
  fi
  i=$((i+1))
done

[ "$DRY_RUN" = 1 ] && exit 0

# Plays read these leaves from $OUT_DIR itself, through the goat_leaf filter
# (ops/ansible/filter_plugins/goat_leaf.py, #2165), so every checkout finds them
# and no private key is copied into a working tree. The filter still accepts a
# pair staged at ops/enrollment/<site>/ by an older run of this script.
echo "mint-site-certs: $SITE — minted=$minted skipped=$skipped out=$OUT_DIR"
echo "Plays read leaves from $OUT_DIR (goat_leaf); staging to hosts stays each role's job."
