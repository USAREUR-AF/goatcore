#!/usr/bin/env bash
#
# goat-zenoh-add-vendor — register one vendor on the trial-fabric router:
# append its ACL entries to the portal-owned ORG FRAGMENT, re-assemble the live
# config from the role-owned SEED + the fragment, and SIGHUP zenohd.
#
# The router loads ONE config.json5. If this script edited it in place AND the
# role re-rendered it, a re-render would silently drop every vendor this script
# had added. So ownership splits:
#   SEED (role)   — config.json5 as the template renders it (base rules + the
#                   zenoh_seed_vendors). Regenerated on every re-render.
#   ORG  (portal) — ONLY the vendors this script / the portal adds at runtime.
#                   Never touched by a re-render.
#   LIVE          — acl-assemble.py assemble(SEED, ORG); what zenohd loads.
# This script now edits the ORG fragment (not the live config) and re-assembles,
# so a subsequent role re-render regenerates the seed and re-splices this
# fragment — the vendor can no longer be clobbered. See docs/findings/F-258.
#
# Idempotent: re-running with the same vendor name is a no-op (+ a reload, in
# case a prior reassemble/SIGHUP failed and we're re-run to recover). The Track
# C portal calls this script (or its equivalent over the zenohd admin API) to
# register vendors programmatically; it is also the operator escape hatch.
#
# The emitted ACL block strings are BYTE-IDENTICAL to the prior in-place-edit
# version — only the destination (fragment vs live config) and the reassemble
# step are new — so a vendor registered today gets exactly the same rules.
#
# Output: structured JSON to stdout on every invocation. Exit codes:
#   0   vendor added or already present + reload succeeded
#   2   bad arguments
#   3   seed or org fragment not found (org is bootstrapped by the role)
#   4   vendor name failed validation
#   5   SIGHUP to zenohd container failed
#   6   fragment edit / assemble failed (markers missing, write-error, etc.)
#
# Usage:
#   goat-zenoh-add-vendor --vendor example-vendor
#   goat-zenoh-add-vendor --vendor example-vendor \
#     --seed /etc/goat-zenoh/config.seed.json5 \
#     --org  /etc/goat-zenoh/acl-org-fragment.json5 \
#     --config /etc/goat-zenoh/config.json5 \
#     --container goat-zenoh-router

set -euo pipefail

SEED_PATH="/etc/goat-zenoh/config.seed.json5"
ORG_PATH="/etc/goat-zenoh/acl-org-fragment.json5"
LIVE_PATH="/etc/goat-zenoh/config.json5"
ASSEMBLER="/usr/local/sbin/goat-zenoh-acl-assemble"
CONTAINER="goat-zenoh-router"
STORAGE_PARENT="/var/lib/goat-zenoh/rocksdb"
NO_RELOAD=0
VENDOR=""

emit() {
  printf '{"vendor":"%s","action":"%s","ok":%s,"reason":"%s"}\n' \
    "${VENDOR}" "$1" "$2" "${3:-}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --vendor)         VENDOR="$2"; shift 2 ;;
    --seed)           SEED_PATH="$2"; shift 2 ;;
    --org)            ORG_PATH="$2"; shift 2 ;;
    --acl|--config)   LIVE_PATH="$2"; shift 2 ;;
    --assembler)      ASSEMBLER="$2"; shift 2 ;;
    --container)      CONTAINER="$2"; shift 2 ;;
    --storage-parent) STORAGE_PARENT="$2"; shift 2 ;;
    # --no-reload: edit the fragment + reassemble the live config on disk but do
    # NOT SIGHUP. The role uses this to bootstrap seed vendors into a fresh
    # fragment before the container is even up; the final assemble + start
    # happens in the role's normal flow.
    --no-reload)      NO_RELOAD=1; shift ;;
    --help|-h)
      cat <<'USAGE'
goat-zenoh-add-vendor --vendor <name> [--seed PATH] [--org PATH] [--config PATH]
                           [--assembler PATH] [--container NAME] [--no-reload]

Appends per-vendor ACL entries (write rule + subject + policy + storage) to the
portal-owned ORG FRAGMENT, re-assembles the live config from the role-owned SEED
+ the fragment, and SIGHUPs zenohd so the policy reloads. Backward-compatible
--acl alias for --config is accepted.

Idempotent. Structured JSON to stdout. Non-zero exit on any failure.
USAGE
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "${VENDOR}" ]]; then
  echo '{"vendor":"","action":"validate","ok":false,"reason":"--vendor is required"}'
  exit 2
fi

# Vendor name validation: lowercase alphanumeric + hyphen, 1-32 chars.
# Rejects path-traversal + JSON-injection + ACL-rule-id collisions structurally.
if ! [[ "${VENDOR}" =~ ^[a-z0-9-]{1,32}$ ]]; then
  emit "validate" false "vendor name must match ^[a-z0-9-]{1,32}$"
  exit 4
fi

if [[ ! -f "${SEED_PATH}" ]]; then
  emit "open-seed" false "seed config not found at ${SEED_PATH}"
  exit 3
fi
if [[ ! -f "${ORG_PATH}" ]]; then
  emit "open-org" false "org fragment not found at ${ORG_PATH} (role init-org bootstraps it)"
  exit 3
fi

reassemble() {
  # Splice SEED + ORG -> LIVE, then reload. Shared by the add path and the
  # already-present recovery path so both self-heal a stale live config.
  if ! "${ASSEMBLER}" assemble --seed "${SEED_PATH}" --org "${ORG_PATH}" --out "${LIVE_PATH}" >/dev/null; then
    emit "assemble" false "acl-assemble assemble failed"
    exit 6
  fi
  chmod 644 "${LIVE_PATH}"
  emit "assemble" true ""
  if [[ "${NO_RELOAD}" -eq 1 ]]; then
    emit "reload" true "skipped (--no-reload)"
    return 0
  fi
  if docker kill --signal HUP "${CONTAINER}" >/dev/null 2>&1; then
    emit "reload" true ""
  else
    emit "reload" false "docker kill --signal HUP ${CONTAINER} failed; live config on disk is updated but the router did not reload"
    exit 5
  fi
}

# Idempotency check — if the vendor's rule id is already in the ORG fragment,
# this is a no-op. Reassemble + reload anyway in case a prior run failed after
# the fragment edit but before the live config was rebuilt/reloaded.
if grep -qF "\"trial-peer-w-${VENDOR}\"" "${ORG_PATH}"; then
  emit "already-present" true ""
  reassemble
  exit 0
fi

# Vendor storage directory must exist before zenohd's rocksdb backend spawns the
# storage (the backend will not create the parent dir). Idempotent.
STORAGE_PATH="${STORAGE_PARENT}/${VENDOR}"
mkdir -p "${STORAGE_PATH}" || { emit "mkdir-storage" false "mkdir -p ${STORAGE_PATH} failed"; exit 6; }

# Atomic-write pattern on the ORG fragment: copy, edit, rename.
TMP="$(mktemp "${ORG_PATH}.XXXXXX")"
trap 'rm -f "${TMP}"' EXIT
cp "${ORG_PATH}" "${TMP}"

# Append the four per-vendor blocks to the correct fragment sections. The
# fragment is a flat file of `##ACL-ORG-REGION <key>` sections; a block is
# inserted at the END of its section (just before the next section header, or
# EOF). The block STRINGS are byte-identical to the prior in-place-edit version
# (INGRESS-only publish grant per ADR 1020; two prefixes per ADR 1032 — the
# persisted <vendor>/** namespace + the non-persisted volatile/<vendor>/** class,
# which gets no storage entry and is audit-excluded). Indentation matches the
# live config's markers (6 spaces for ACL blocks, 8 for the storage block) so an
# assemble splices them back byte-exact.
python3 - "${TMP}" "${VENDOR}" <<'PY' || { emit "edit-org" false "python3 fragment edit failed"; exit 6; }
import sys, pathlib
path = pathlib.Path(sys.argv[1])
vendor = sys.argv[2]

rule = f"""      {{
        id: "trial-peer-w-{vendor}",
        messages: ["put", "delete"],
        flows: ["ingress"],
        permission: "allow",
        key_exprs: ["{vendor}/**", "volatile/{vendor}/**"],
      }},
"""
subject = f"""      {{
        id: "vendor-{vendor}",
        cert_common_names: ["{vendor}-trial-peer"],
      }},
"""
policy = f"""      {{
        rules: ["trial-peer-w-{vendor}", "trial-peer-w-attest-schema"],
        subjects: ["vendor-{vendor}"],
      }},
"""
storage = f"""        "{vendor}": {{
          key_expr: "{vendor}/**",
          volume: {{
            id: "rocksdb",
            dir: "/var/lib/goat-zenoh/rocksdb/{vendor}",
            on_closure: "do_nothing",
            create_db: true,
            db_recovery_enabled: false,
          }},
          garbage_collect_period: 86400,
          time_to_live: 604800000,
        }},
"""

# Fragment section header prefix (mirrors acl-assemble.py FRAG_HDR).
HDR = "##ACL-ORG-REGION "
blocks = {"write-rules": rule, "subjects": subject, "policies": policy, "storages": storage}

lines = path.read_text().splitlines(keepends=True)
# All section header positions, in file order.
headers = [(i, ln[len(HDR):].strip()) for i, ln in enumerate(lines) if ln.startswith(HDR)]
present = {k for _i, k in headers}
missing = [k for k in blocks if k not in present]
if missing:
    sys.stderr.write(f"org fragment missing section header(s): {missing}\n")
    sys.exit(1)

# A block goes at the END of its section = just before the next header after its
# own, or EOF. Apply from the BOTTOM up so earlier indices stay valid.
def section_end(key):
    hdr_i = next(i for i, k in headers if k == key)
    after = [i for i, _k in headers if i > hdr_i]
    return after[0] if after else len(lines)

inserts = sorted(((section_end(k), blocks[k]) for k in blocks), key=lambda t: t[0], reverse=True)
for at, block in inserts:
    lines[at:at] = [block]
path.write_text("".join(lines))
PY

mv "${TMP}" "${ORG_PATH}"
chmod 644 "${ORG_PATH}"
trap - EXIT
emit "edit-org" true ""

# Rebuild the live config from SEED + the updated fragment, then reload.
reassemble
exit 0
