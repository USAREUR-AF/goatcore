#!/usr/bin/env bash
# check-no-live-secret.sh — refuse to commit a value that is a CURRENTLY-LIVE
# secret, by hash-comparing content against the operator's own environment.
#
# WHY THIS EXISTS, and why it is separate from every other guard here.
# GitHub push protection and secret scanning (enabled) catch secrets
# with a KNOWN SHAPE — AWS keys, GitHub tokens, provider patterns. They cannot
# catch NB_ENCKEY, BKP_KEY, a Postgres password or a portal operator key,
# because those are bare high-entropy strings with no distinguishing pattern.
# check-no-identity-spillage.sh catches age recipients and SSH pubkeys by shape.
# Nothing catches "this base64 blob is, byte for byte, the encryption key a live
# goatnet is using right now."
#
# the pre-open-sourcing history triage found exactly that: an e2e
# fixture (netbird-dashboard/tests/fixture-e2e/seed.sh) carried a
# DataStoreEncryptionKey identical to the live NB_ENCKEY of four goatnets, and a
# deleted tee-up doc carried the live portal operator key. Neither has a
# recognisable shape; both would have sailed through every shape-based guard.
# NB_ENCKEY in particular has no rotation path, so its leak is
# permanent — which is why the operator's instruction was specifically that a
# real environment's NB_ENCKEY must never be committed.
#
# THE ONE PLACE THIS CAN RUN is where the live values are: the operator laptop,
# via direnv. CI cannot run it — the CI runner has no .envrc.local.d and must
# not. So this is a PRE-COMMIT / operator-laptop control by construction, and it
# says so loudly when the environment is absent rather than pretending to pass.
#
# It compares by SHA-256. No secret value is ever read into a variable that is
# printed, logged, or placed on a command line; only digests are compared, and
# only the VARIABLE NAME of a match is ever emitted.
#
# Usage:
#   ops/ci/check-no-live-secret.sh --staged     # pre-commit: scan the staged diff
#   ops/ci/check-no-live-secret.sh <path>...    # scan named files
#   ops/ci/check-no-live-secret.sh              # scan the whole tracked tree
#
# Exit: 0 clean (or environment absent — see below) · 1 a live secret is present
#       · 2 usage error.

set -uo pipefail

MIN_LEN=16   # shorter env values are not credentials; comparing them invites
             # false positives on common words that happen to match a short var.

# --- is this value credential-SHAPED, or is it a name? -----------------------
# Length alone does not separate them. `$ZAGROS_AUDIT_SUBSCRIBER_S3_ACCESS_KEY`
# holds a 16-character hyphenated lowercase SERVICE NAME — the same word that
# names the container, the scrape job, the rule file and the finding — so it
# clears MIN_LEN and appears legitimately in prose all over the repo. On
# it refused an unrelated edit to a group_vars comment listing
# scrape ports, and refused every edit to docs/findings/INDEX.md, in two
# different sessions on the same day. A guard that cannot be satisfied is a
# guard people learn to pass with --no-verify, which is worse than not having
# one. Diff-scoping would not have helped: in both cases the word was on a line
# the author genuinely added.
#
# THE TEST, and why it cannot skip a real secret. A value is treated as a NAME
# rather than a credential when all three hold:
#
#   * it is entirely lowercase [a-z0-9_-], and
#   * it contains at least one `-` or `_` separator, and
#   * some separated segment contains a letter outside [a-f].
#
# Each clause is load-bearing. All-lowercase keeps base64 (mixed case) and
# AGE-SECRET-KEY-… (uppercase) protected. Requiring a separator keeps bare hex
# and base64 digests, which have none. Requiring a non-hex letter keeps UUIDs
# and hyphen-grouped hex, whose segments are hex-only. What is left — a
# lowercase word with hyphens containing letters like r, s or t — is not the
# output of any generator this repo uses, and grep could never have protected
# it anyway, because it is *supposed* to appear in prose.
#
# Skipped values are counted and named at the end, never silently dropped. A
# credential whose value is a guessable service name is weak independent of
# this scanner, and the operator should be told which ones are in that state.
# MAX_NAME_SEGMENT bounds how long one piece of a "name" may be. Words are
# short. This clause is what stops the filter unprotecting a strong secret that
# merely happens to be lowercase: `SNITCH_ADMIN_KEY` is a short prefix, an
# underscore, and a 64-character random lowercase segment — all-lowercase, with
# a separator, with non-hex letters, and therefore a NAME by the first three
# clauses alone. It is not a name; it is roughly 330 bits of entropy. The three
# SNITCH_* keys and both PROVENANCE_GUARD_PORTAL_KEYs were briefly unprotected
# for exactly that reason. Twenty-four is well above any real segment
# (`auditreader` is 11, `subscriber` is 10) and far below a generated one.
MAX_NAME_SEGMENT=24

# A variable whose NAME matches this is a candidate secret. Defined here once,
# with MIN_LEN, MAX_NAME_SEGMENT and is_name_shaped, so another script reads all
# of them from this file: ops/goatbuilder/c3-measure.sh scrubs these values from
# its record.
# This variable's own name matches none of it, so the pattern never scans for itself.
CANDIDATE_NAME_RE='(PASS|PASSWORD|SECRET|TOKEN|KEY|ENCKEY|PASSPHRASE|_PW$|CIPHER|COOKIE|CSRF|SALT|CREDENTIAL|REPLICATOR|API)'

is_name_shaped() {
  local v=$1
  [[ $v =~ ^[a-z0-9_-]+$ ]] || return 1           # any uppercase / +/= → credential
  [[ $v == *-* || $v == *_* ]] || return 1        # no separator → credential
  local seg has_word=1
  while IFS= read -r seg; do
    # One long segment is enough to disqualify the whole value: a generated
    # secret with a human prefix (`snitch_<64 random>`) must stay protected.
    (( ${#seg} > MAX_NAME_SEGMENT )) && return 1
    [[ $seg =~ [g-z] ]] && has_word=0             # a non-hex letter → word-like
  done < <(printf '%s\n' "${v//[-_]/$'\n'}")
  return $has_word                                # 0 = name; 1 = every segment hex-only
}

# --- collect the live secret digests ----------------------------------------
# A value is a candidate if its variable name looks secret-bearing. We hash
# values, never store them, into a "digest name" table on a private tempfile.
# A plain file, not an associative array: this runs as a pre-commit hook under
# whatever `/usr/bin/env bash` resolves to, and macOS ships bash 3.2 which has
# no `declare -A`. A guard that errors on the operator's own shell is a guard
# wired to nothing.
DIGESTS=$(mktemp "${TMPDIR:-/tmp}/.live-digests.XXXXXX") || { echo "cannot mktemp" >&2; exit 2; }
chmod 600 "$DIGESTS"
trap 'rm -f "$DIGESTS"' EXIT
live_count=0
weak_names=()
# Values reach the hasher over a PIPE, never a file: the digest table is what
# lands on disk (mode 600), the plaintext values do not. One hashing process for
# all of them rather than one `shasum` per variable — with ~340 secret-bearing
# variables set on the operator laptop, the per-variable spawn was most of this
# guard's startup cost every single commit.
hash_values() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c '
import hashlib, sys
data = sys.stdin.buffer.read().split(b"\0")
for i in range(0, len(data) - 1, 2):
    name, val = data[i], data[i + 1]
    if val:
        sys.stdout.write(hashlib.sha256(val).hexdigest() + " " + name.decode() + "\n")
'
  else
    # Fallback: the original per-value shasum, slower but identical in effect.
    local name val
    while IFS= read -r -d '' name && IFS= read -r -d '' val; do
      [[ -n "$val" ]] && printf '%s %s\n' \
        "$(printf '%s' "$val" | shasum -a 256 | cut -d' ' -f1)" "$name"
    done
  fi
}

# Pass 1 — classify, in THIS shell so the counters survive. Names only; no
# value is copied anywhere here.
cred_names=()
while IFS= read -r name; do
  eval "val=\${$name-}"
  [[ -z "$val" || ${#val} -lt $MIN_LEN ]] && continue
  if is_name_shaped "$val"; then
    weak_names+=("$name")
    continue
  fi
  cred_names+=("$name")
  live_count=$((live_count+1))
done < <(compgen -v | grep -E "$CANDIDATE_NAME_RE" )

# Pass 2 — hash them. This half may run in a subshell: it sets nothing, it only
# writes the digest table.
if (( live_count > 0 )); then
  for name in "${cred_names[@]}"; do
    eval "val=\${$name-}"
    printf '%s\0%s\0' "$name" "$val"
  done | hash_values >> "$DIGESTS"
fi

if (( live_count == 0 )); then
  echo "check-no-live-secret: no secret-bearing environment variables are set."
  echo "  This guard compares committed content against the operator's OWN live"
  echo "  secrets, so it only works on a laptop with .envrc.local.d loaded via"
  echo "  direnv. In CI this is expected and correct — CI holds no live secrets."
  echo "  SKIPPING (not a pass): run it from the repo root with direnv active."
  exit 0
fi

# --- pick the files to scan -------------------------------------------------
mode=tree
if [[ "${1:-}" == "--staged" ]]; then mode=staged; shift
elif [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then sed -n '2,40p' "$0"; exit 0; fi
paths=("$@")

files=()
while IFS= read -r f; do [[ -n "$f" ]] && files+=("$f"); done < <(
  if [[ "$mode" == "staged" ]]; then git diff --cached --name-only --diff-filter=ACMR
  elif (( ${#paths[@]} )); then printf '%s\n' "${paths[@]}"
  else git ls-files; fi
)

# An empty set is a normal case, not an error: `git commit --amend` with nothing
# newly staged reaches here with zero files. Say so and stop. This matters on
# macOS, where the pre-commit hook runs under /bin/bash 3.2: that bash treats
# "${files[@]}" on an EMPTY array as an unbound variable under `set -u` (fixed
# only in bash 4.4). Left unguarded, the python path below printed
# "files[@]: unbound variable" and then reported ok for a scan it never ran,
# and the no-python fallback aborted the whole hook (GH #967). The two
# expansions further down are also written in the 3.2-safe
# ${files[@]+"${files[@]}"} form so this file never carries a bare one again.
# Defined here rather than at the bottom so the empty-set return below can
# print it too: the NOTE is about the credentials, not about the scan, and a
# commit that scanned nothing still deserves the reminder.
report_weak_names() {
  (( ${#weak_names[@]} == 0 )) && return 0
  echo
  echo "NOTE — ${#weak_names[@]} secret-bearing variable(s) hold a value this guard cannot"
  echo "protect, because the value is a lowercase hyphenated word rather than a"
  echo "generated credential. Such a value appears legitimately in prose, so"
  echo "matching on it refuses ordinary edits; it is skipped rather than enforced:"
  printf '  %s\n' "${weak_names[@]}"
  echo
  echo "This is a finding about the credentials, not about the scanner. A value a"
  echo "reader could guess from a container or job name is weak whatever this"
  echo "guard does. Rotating each to a generated value (openssl rand -base64 32)"
  echo "fixes the credential AND brings it back under this guard's protection."
}

if (( ${#files[@]} == 0 )); then
  echo "ok: nothing to scan — the $mode file set is empty ($live_count live values loaded, 0 files checked)"
  report_weak_names
  exit 0
fi

# --- scan -------------------------------------------------------------------
# Pull every base64-ish and hex-ish token of plausible length from each file,
# hash it, and see if the digest is one of ours. Reading is per-file; matched
# tokens are never printed.
violations=()

# ONE hashing process for the whole run, not one per token.
#
# The original loop spawned `shasum` for every candidate token in every scanned
# file. As a pre-commit hook that made the cost scale with the SIZE of the files
# touched rather than the size of the change: a commit including
# docs/adr/INDEX.md (450 KB) took minutes, and a scan of the whole tree did not
# finish inside two. A guard slow enough to interrupt a commit gets bypassed for
# the same reason an unsatisfiable one does, so speed is correctness here.
#
# python3 is already a hard dependency of this repo's tooling (gen-roster.py,
# generate-mesh-hosts.py, the identity generator). If it is somehow absent the
# per-token bash path below still runs, so the guard degrades in speed rather
# than in coverage.
scan_with_python() {
  python3 - "$DIGESTS" "$mode" ${files[@]+"${files[@]}"} <<'PY'
import hashlib, re, subprocess, sys

digests_path, mode, files = sys.argv[1], sys.argv[2], sys.argv[3:]
digests = {}
with open(digests_path) as fh:
    for line in fh:
        d, _, name = line.strip().partition(' ')
        if d:
            digests[d] = name

TOKEN = re.compile(rb'[A-Za-z0-9+/=_-]{16,}')
seen = set()
for f in files:
    if mode == 'staged':
        p = subprocess.run(['git', 'show', f':{f}'], capture_output=True)
        if p.returncode != 0:
            continue
        blob = p.stdout
    else:
        try:
            with open(f, 'rb') as fh:
                blob = fh.read()
        except OSError:
            continue
    if b'\0' in blob[:8000]:      # binary
        continue
    for tok in set(TOKEN.findall(blob)):
        name = digests.get(hashlib.sha256(tok).hexdigest())
        if name and (f, name) not in seen:
            seen.add((f, name))
            # The token itself is never printed.
            print(f'{f} — matches live ${name}')
PY
}

if command -v python3 >/dev/null 2>&1; then
  while IFS= read -r v; do [[ -n "$v" ]] && violations+=("$v"); done < <(scan_with_python)
else
  for f in ${files[@]+"${files[@]}"}; do
    [[ -f "$f" ]] || continue
    # skip binary
    if LC_ALL=C grep -qI . "$f" 2>/dev/null; then :; else continue; fi
    # staged mode compares the STAGED blob, not the worktree file
    content() { if [[ "$mode" == "staged" ]]; then git show ":$f" 2>/dev/null; else cat "$f"; fi; }
    while IFS= read -r tok; do
      [[ ${#tok} -lt $MIN_LEN ]] && continue
      d=$(printf '%s' "$tok" | shasum -a 256 | cut -d' ' -f1)
      hit=$(grep "^$d " "$DIGESTS" | head -1 | cut -d' ' -f2)
      if [[ -n "$hit" ]]; then
        violations+=("$f — matches live \$$hit")
      fi
    done < <(content | grep -oE '[A-Za-z0-9+/=_-]{16,}' | sort -u)
  done
fi

if (( ${#violations[@]} == 0 )); then
  echo "ok: no live secret value present in the scanned content ($live_count live values checked)"
  report_weak_names
  exit 0
fi

echo "REFUSED: content matches a currently-live secret, byte for byte." >&2
echo >&2
for v in "${violations[@]}"; do echo "  $v" >&2; done
echo >&2
echo "This is a real credential, not a fixture. A fixture needs A value, not THE" >&2
echo "value — generate one (openssl rand -base64 32). If a live secret has already" >&2
echo "reached a commit, it must be rotated, not just removed; and NB_ENCKEY cannot" >&2
echo "be rotated at all, so it must never land in the first place." >&2
echo "Guidance: docs/design/credential-hygiene-strategy.md" >&2
exit 1
