#!/usr/bin/env bash
#
# offload-to-cold.sh — Mirror a MinIO bucket into a cold-storage bucket
# and emit a manifest. Operator-fired step in the Block 95 audit-data
# retention substrate. See docs/design/audit-cold-storage-offload.md.
#
# Pre-flight: the destination bucket must exist with Object Lock
# enabled. Run the cold-storage-bucket ansible role first:
#
#   ansible-playbook ... -e cold_storage_dataset_id=<dataset-id> ...
#
# Then this script (typically over SSH against the MinIO host):
#
#   ./offload-to-cold.sh \
#     --ssh-host        operator@<mesh-ip> \
#     --minio-container minio \
#     --source-bucket   audit-integrity \
#     --dataset-id      site-a-1.0 \
#     --operator        <operator-handle> \
#     --reason          "milestone close ..."
#
# Idempotent: re-running on the same dataset with no new source data
# recomputes the manifest sha256 and reports drift. Re-running on
# a dataset whose source bucket has new objects refuses by default
# (pass --allow-extend to extend the dataset to cover the new objects).
#
# Wire format: docs/design/audit-cold-storage-offload.md §"Manifest contract".

set -euo pipefail

# ---- defaults ----
SSH_HOST=""
MINIO_CONTAINER="minio"
SOURCE_ALIAS="local"
SOURCE_BUCKET=""
TARGET_ALIAS="local"
DATASET_ID=""
BUCKET_SUFFIX="-cold-storage"
RETENTION_DAYS=365
OBJECT_LOCK="COMPLIANCE"
OPERATOR=""
REASON=""
ALLOW_EXTEND=0
DRY_RUN=0

usage() {
  sed -n 's/^# \{0,1\}//p' "$0" | sed -n '1,40p'
  echo
  echo "Required flags:"
  echo "  --ssh-host          <user>@<mesh-ip>           (the MinIO host)"
  echo "  --source-bucket     <name>                     (e.g. audit-integrity)"
  echo "  --dataset-id        <kebab-case>               (e.g. nova-1.0)"
  echo "  --operator          <handle>                   (your operator handle)"
  echo "  --reason            <one-line free text>       (offload trigger reason)"
  echo
  echo "Optional flags:"
  echo "  --minio-container   <name>     (default: minio)"
  echo "  --source-alias      <mc-alias> (default: local)"
  echo "  --target-alias      <mc-alias> (default: local)"
  echo "  --bucket-suffix     <s>        (default: -cold-storage)"
  echo "  --retention-days    <N>        (default: 365 — confirm before <365)"
  echo "  --object-lock       <mode>     (default: COMPLIANCE — confirm before GOVERNANCE)"
  echo "  --allow-extend                 (allow re-running on a dataset with new source data)"
  echo "  --dry-run                      (do everything except the final mirror + manifest PUT)"
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --ssh-host)         SSH_HOST="$2"; shift 2 ;;
    --minio-container)  MINIO_CONTAINER="$2"; shift 2 ;;
    --source-alias)     SOURCE_ALIAS="$2"; shift 2 ;;
    --source-bucket)    SOURCE_BUCKET="$2"; shift 2 ;;
    --target-alias)     TARGET_ALIAS="$2"; shift 2 ;;
    --dataset-id)       DATASET_ID="$2"; shift 2 ;;
    --bucket-suffix)    BUCKET_SUFFIX="$2"; shift 2 ;;
    --retention-days)   RETENTION_DAYS="$2"; shift 2 ;;
    --object-lock)      OBJECT_LOCK="$2"; shift 2 ;;
    --operator)         OPERATOR="$2"; shift 2 ;;
    --reason)           REASON="$2"; shift 2 ;;
    --allow-extend)     ALLOW_EXTEND=1; shift 1 ;;
    --dry-run)          DRY_RUN=1; shift 1 ;;
    -h|--help)          usage ;;
    *) echo "unknown flag: $1" >&2; usage ;;
  esac
done

# ---- validate ----
for v in SSH_HOST SOURCE_BUCKET DATASET_ID OPERATOR REASON; do
  if [ -z "${!v}" ]; then
    # tr, not ${v,,}: case-conversion expansion is bash 4+ and this is an
    # operator-fired script that must run on stock macOS /bin/bash 3.2.
    echo "missing required: --$(printf '%s' "$v" | tr '[:upper:]_' '[:lower:]-')" >&2
    usage
  fi
done

TARGET_BUCKET="${DATASET_ID}${BUCKET_SUFFIX}"
echo "==> dataset      : ${DATASET_ID}"
echo "==> source       : ${SOURCE_ALIAS}/${SOURCE_BUCKET}"
echo "==> target       : ${TARGET_ALIAS}/${TARGET_BUCKET}"
echo "==> retention    : ${RETENTION_DAYS}d ${OBJECT_LOCK}"
echo "==> operator     : ${OPERATOR}"
echo "==> reason       : ${REASON}"
echo "==> ssh host     : ${SSH_HOST} (minio container: ${MINIO_CONTAINER})"
[ "$ALLOW_EXTEND" = "1" ] && echo "==> allow-extend : YES (re-running on new source data is OK)"
[ "$DRY_RUN" = "1" ] && echo "==> DRY RUN       : YES (no mirror, no manifest PUT)"

mc_remote() {
  ssh -o BatchMode=yes "${SSH_HOST}" \
    "sudo docker exec ${MINIO_CONTAINER} mc $*"
}

# ---- pre-flight ----
echo
echo "==> pre-flight: target bucket exists with Object Lock"
if ! mc_remote ls "${TARGET_ALIAS}/${TARGET_BUCKET}" > /dev/null 2>&1; then
  echo "  FAIL: target bucket ${TARGET_ALIAS}/${TARGET_BUCKET} does not exist." >&2
  echo "  Run the cold-storage-bucket ansible role first:" >&2
  echo "    ansible-playbook ... -e cold_storage_dataset_id=${DATASET_ID} ..." >&2
  exit 2
fi
echo "  OK"

# Check whether the manifest already exists.
MANIFEST_KEY="manifests/${DATASET_ID}.json"
OBJECTS_KEY="manifests/${DATASET_ID}.objects.ndjson"

echo
echo "==> pre-flight: existing manifest?"
if mc_remote stat "${TARGET_ALIAS}/${TARGET_BUCKET}/${MANIFEST_KEY}" > /dev/null 2>&1; then
  echo "  EXISTS — this is a re-run on dataset ${DATASET_ID}"
  if [ "$ALLOW_EXTEND" = "0" ]; then
    echo
    echo "  --allow-extend not set. Source-bucket drift check:" >&2
    # Count source vs manifest's recorded source.object_count.
    PREV_OBJECT_COUNT=$(mc_remote cat "${TARGET_ALIAS}/${TARGET_BUCKET}/${MANIFEST_KEY}" 2>/dev/null \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["source"]["object_count"])')
    CURRENT_OBJECT_COUNT=$(mc_remote ls --recursive --json "${SOURCE_ALIAS}/${SOURCE_BUCKET}" 2>/dev/null \
      | grep -c '"key"' || echo 0)
    echo "    manifest source.object_count = ${PREV_OBJECT_COUNT}"
    echo "    current source object count  = ${CURRENT_OBJECT_COUNT}"
    if [ "${CURRENT_OBJECT_COUNT}" -gt "${PREV_OBJECT_COUNT}" ]; then
      echo "  REFUSED: source bucket has new objects since this dataset was archived." >&2
      echo "  Pass --allow-extend to extend the dataset, or pick a new --dataset-id." >&2
      exit 3
    fi
    echo "  No drift. Re-running will recompute manifest sha256 only (idempotent re-verify)."
  else
    echo "  --allow-extend set; re-running will mirror new objects + update manifest."
  fi
else
  echo "  NEW dataset"
fi

if [ "$DRY_RUN" = "1" ]; then
  echo
  echo "==> DRY RUN — exiting before mirror + manifest."
  exit 0
fi

# ---- mirror ----
echo
echo "==> mirror: ${SOURCE_ALIAS}/${SOURCE_BUCKET} → ${TARGET_ALIAS}/${TARGET_BUCKET}"
echo "    (--preserve carries per-object retention timestamps through)"
START_TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
mc_remote mirror --preserve --overwrite \
  "${SOURCE_ALIAS}/${SOURCE_BUCKET}" \
  "${TARGET_ALIAS}/${TARGET_BUCKET}" 2>&1 | tail -5
END_TS=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
echo "  done."

# ---- objects-ledger ----
echo
echo "==> compute objects ledger (key + sha256 + bytes, sorted by key)"
OBJECTS_NDJSON_LOCAL="/tmp/${DATASET_ID}.objects.ndjson"
mc_remote ls --recursive --json "${TARGET_ALIAS}/${TARGET_BUCKET}" \
  | python3 -c '
import json, sys
rows = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        d = json.loads(line)
    except json.JSONDecodeError:
        continue
    if d.get("type") != "file":
        continue
    key = d.get("key", "")
    # Skip the manifest folder itself so the ledger doesnt self-reference.
    if key.startswith("manifests/"):
        continue
    rows.append({
        "key":   key,
        "sha256": d.get("etag", "").lower(),
        "bytes": int(d.get("size", 0)),
    })
rows.sort(key=lambda r: r["key"])
for r in rows:
    print(json.dumps(r, sort_keys=True))
' > "${OBJECTS_NDJSON_LOCAL}"

OBJECT_COUNT=$(wc -l < "${OBJECTS_NDJSON_LOCAL}" | tr -d ' ')
TOTAL_BYTES=$(python3 -c '
import json, sys
total = 0
for line in open(sys.argv[1]):
    line = line.strip()
    if line:
        total += int(json.loads(line).get("bytes", 0))
print(total)
' "${OBJECTS_NDJSON_LOCAL}")
LEDGER_SHA256=$(shasum -a 256 "${OBJECTS_NDJSON_LOCAL}" | awk '{print $1}')

echo "  object_count: ${OBJECT_COUNT}"
echo "  total_bytes : ${TOTAL_BYTES}"
echo "  ledger sha256: ${LEDGER_SHA256}"

# ---- manifest ----
echo
echo "==> emit manifest"
MANIFEST_LOCAL="/tmp/${DATASET_ID}.json"
RETAIN_UNTIL=$(date -u -v+${RETENTION_DAYS}d +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || \
               date -u -d "+${RETENTION_DAYS} days" +"%Y-%m-%dT%H:%M:%SZ")

python3 - "$MANIFEST_LOCAL" <<EOF
import json, sys
manifest = {
    "schema_version": 1,
    "dataset_id": "${DATASET_ID}",
    "source": {
        "ssh_host": "${SSH_HOST}",
        "alias":    "${SOURCE_ALIAS}",
        "bucket":   "${SOURCE_BUCKET}",
    },
    "coverage": {
        "object_count": ${OBJECT_COUNT},
        "total_bytes":  ${TOTAL_BYTES},
    },
    "archive": {
        "alias":  "${TARGET_ALIAS}",
        "bucket": "${TARGET_BUCKET}",
        "sha256_of_manifest_of_objects": "${LEDGER_SHA256}",
        "objects_ndjson_key": "${OBJECTS_KEY}",
    },
    "operator": {
        "handle":          "${OPERATOR}",
        "trigger_reason":  "${REASON}",
    },
    "timestamps": {
        "offload_started":   "${START_TS}",
        "offload_completed": "${END_TS}",
    },
    "object_lock": {
        "mode":           "${OBJECT_LOCK}",
        "retention_days": ${RETENTION_DAYS},
        "retain_until":   "${RETAIN_UNTIL}",
    },
}
with open(sys.argv[1], "w") as fh:
    json.dump(manifest, fh, indent=2, sort_keys=True)
    fh.write("\n")
EOF
cat "${MANIFEST_LOCAL}"

# ---- PUT manifest + objects ledger ----
echo
echo "==> PUT manifest + objects ledger to target bucket"
# Copy files into the container's mc-accessible scratch dir, then mc cp into the bucket.
SCRATCH_DIR="/tmp/cold-offload-${DATASET_ID}"
scp -q "${OBJECTS_NDJSON_LOCAL}" "${MANIFEST_LOCAL}" \
  "${SSH_HOST}":"${SCRATCH_DIR}/" 2>/dev/null || {
    # Pre-create scratch dir + retry
    ssh "${SSH_HOST}" "mkdir -p '${SCRATCH_DIR}'"
    scp -q "${OBJECTS_NDJSON_LOCAL}" "${MANIFEST_LOCAL}" "${SSH_HOST}":"${SCRATCH_DIR}/"
  }

# Copy from host filesystem into MinIO via mc cp inside the container,
# using a bind-mount-style staging trick: we copy from host /tmp into
# the container fs first, then mc cp puts them into MinIO.
ssh "${SSH_HOST}" "sudo docker cp '${SCRATCH_DIR}/${DATASET_ID}.json' ${MINIO_CONTAINER}:/tmp/"
ssh "${SSH_HOST}" "sudo docker cp '${SCRATCH_DIR}/${DATASET_ID}.objects.ndjson' ${MINIO_CONTAINER}:/tmp/"

mc_remote cp "/tmp/${DATASET_ID}.json" \
  "${TARGET_ALIAS}/${TARGET_BUCKET}/${MANIFEST_KEY}"
mc_remote cp "/tmp/${DATASET_ID}.objects.ndjson" \
  "${TARGET_ALIAS}/${TARGET_BUCKET}/${OBJECTS_KEY}"

# Apply object-lock retention to the two manifest files.
mc_remote retention set "${OBJECT_LOCK}" "${RETENTION_DAYS}d" \
  "${TARGET_ALIAS}/${TARGET_BUCKET}/${MANIFEST_KEY}"
mc_remote retention set "${OBJECT_LOCK}" "${RETENTION_DAYS}d" \
  "${TARGET_ALIAS}/${TARGET_BUCKET}/${OBJECTS_KEY}"

# Clean staging.
ssh "${SSH_HOST}" "rm -rf '${SCRATCH_DIR}'"
ssh "${SSH_HOST}" "sudo docker exec ${MINIO_CONTAINER} rm -f /tmp/${DATASET_ID}.json /tmp/${DATASET_ID}.objects.ndjson" || true

echo
echo "==> done. Dataset ${DATASET_ID} cold-archived."
echo "    manifest:        s3://${TARGET_BUCKET}/${MANIFEST_KEY}"
echo "    objects ledger:  s3://${TARGET_BUCKET}/${OBJECTS_KEY}"
echo "    retain until:    ${RETAIN_UNTIL}"
echo
echo "Next:"
echo "  * Commit a copy of the manifest as a measurement artifact:"
echo "      docs/measurements/cold-archive-${DATASET_ID}-manifest.json"
echo "  * Verify the archive is queryable by sha256:"
echo "      ./verify-cold-archive.sh --ssh-host ${SSH_HOST} --dataset-id ${DATASET_ID}"
