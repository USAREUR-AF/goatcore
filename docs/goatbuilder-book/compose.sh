#!/usr/bin/env bash
# compose.sh — build the goatbuilder book's composed chapters from the
# canonical files, so the book never forks a document.
#
#   docs/goatbuilder-book/compose.sh <path to a goatbuilder binary>
#
# Writes src/SUMMARY.md and src/composed/ (both gitignored — the table of
# contents links chapters that exist only after composing, and the docs link
# gate reads every committed .md):
#   reference/*.md               <- docs/reference/goatbuilder/*.md (the reference)
#   reference/figures/*.svg      <- docs/reference/goatbuilder/figures/
#   reference/appendix-a-error-codes.md gets its table from `goatbuilder explain`
#   reference/appendix-d-commands.md <- every verb's --help, verbs read from --help
#   first-run.md                 <- docs/operations/goatbuilder-first-run.md
#   goatbuilder.md               <- docs/design/goatbuilder.md
#   goatbuilder-conformance.md   <- docs/design/goatbuilder-conformance.md
#   goatbuilder-distribution.md  <- docs/design/goatbuilder-distribution.md
#   tutorial-1-new-goatnet.md    <- docs/tutorials/01-new-goatnet.md
#   tutorial-2-first-federated-node.md <- docs/tutorials/02-first-federated-node.md
#   goatnet-tutorials.md         <- docs/design/goatnet-tutorials.md
#   reference.md                 <- `goatbuilder --help` + every verb's --help
#
# Relative links between repository documents (../adr/…, ../findings/…,
# ../../ops/…) point at files this book does not carry; they are rendered as
# their link text so a reader is never sent to a 404. Links to the other
# composed chapters are rewritten to the composed path. Run from any cwd.
set -euo pipefail

BIN="${1:?usage: compose.sh <goatbuilder binary>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$HERE/src/composed"
mkdir -p "$OUT" "$HERE/theme"
# The goat-cli book's stylesheet, copied not forked: mdBook serves only files
# under the book root, so the one stylesheet is copied in at build time.
cp "$HERE/../goat-cli-book/theme/goat.css" "$HERE/theme/goat.css"

compose() {
  # $1 source (repo-relative), $2 destination name
  python3 - "$ROOT/$1" "$OUT/$2" <<'PY'
import re, sys
src, dst = sys.argv[1], sys.argv[2]
text = open(src, encoding="utf-8").read()
carried = {
    "goatbuilder.md": "goatbuilder.md",
    "goatbuilder-conformance.md": "goatbuilder-conformance.md",
    "goatbuilder-distribution.md": "goatbuilder-distribution.md",
    "goatbuilder-first-run.md": "first-run.md",
    "01-new-goatnet.md": "tutorial-1-new-goatnet.md",
    "02-first-federated-node.md": "tutorial-2-first-federated-node.md",
    "goatnet-tutorials.md": "goatnet-tutorials.md",
}
in_ref = "/reference/goatbuilder/" in src
up = "../" if in_ref else ""
def fix(m):
    label, target = m.group(1), m.group(2)
    if target.startswith(("http://", "https://", "#")):
        return m.group(0)
    path = target.split("#")[0]
    base = path.rsplit("/", 1)[-1]
    frag = target[len(path):]
    if in_ref and "/" not in path and base.endswith(".md"):
        return m.group(0)                      # a sibling chapter of the reference
    if "/reference/goatbuilder/" in "/" + path or (not in_ref and path.startswith("../reference/goatbuilder/")):
        return "[%s](reference/%s%s)" % (label, "01-what-it-is.md" if base == "README.md" else base, frag)
    if base in carried:
        return "[%s](%s%s%s)" % (label, up, carried[base], frag)
    return label
text = re.sub(r"(?<!!)\[([^\]]+)\]\(([^)\s]+)\)", fix, text)
open(dst, "w", encoding="utf-8").write(text)
PY
}

REF="$ROOT/docs/reference/goatbuilder"
mkdir -p "$OUT/reference/figures"
# The figures directory is optional: a tree that carries no figures builds the
# book without them.
if compgen -G "$REF/figures/*.svg" >/dev/null; then
  cp "$REF"/figures/*.svg "$OUT/reference/figures/"
fi

# The table of contents: the reference in its own order (its README lists the
# chapters), then the generated command reference, then the guides and design
# documents the reference cites.
{
  echo "# Summary"
  echo
  echo "[Welcome](introduction.md)"
  echo
  echo "# Reference"
  echo
  python3 - "$REF/README.md" <<'PY'
import re, sys
for line in open(sys.argv[1], encoding="utf-8"):
    m = re.match(r"\| *([0-9A-Z]+) *\| *\[([^\]]+)\]\(([^)]+)\) *\|", line)
    if m:
        n, title, f = m.groups()
        label = ("%s. %s" % (n, title)) if n.isdigit() else ("Appendix %s. %s" % (n, title))
        print("- [%s](composed/reference/%s)" % (label, f))
PY
  echo "- [Appendix D. Command reference](composed/reference/appendix-d-commands.md)"
  echo
  echo "# Guides and design"
  echo
  echo "- [First run](composed/first-run.md)"
  echo "- [Tutorial 1: a new goatnet](composed/tutorial-1-new-goatnet.md)"
  echo "- [Tutorial 2: the first federated node](composed/tutorial-2-first-federated-node.md)"
  echo "- [The tutorial program](composed/goatnet-tutorials.md)"
  echo "- [The design: declare, converge, verify](composed/goatbuilder.md)"
  echo "- [The contract as tests](composed/goatbuilder-conformance.md)"
  echo "- [Distribution and hosting](composed/goatbuilder-distribution.md)"
} > "$HERE/src/SUMMARY.md"

for f in "$REF"/[0-9][0-9]-*.md "$REF"/appendix-*.md; do
  compose "docs/reference/goatbuilder/$(basename "$f")" "reference/$(basename "$f")"
done

# Appendix A's table, from the catalog compiled into this binary.
python3 - "$BIN" "$OUT/reference/appendix-a-error-codes.md" <<'PY'
import json, re, subprocess, sys
bin_, dst = sys.argv[1], sys.argv[2]
listing = subprocess.run([bin_, "--lang", "en", "explain", "--list"], capture_output=True, text=True, check=True).stdout
codes = [l.split()[0] for l in listing.splitlines() if re.match(r"(GB-\d{3}|X-[A-Z]+-\d{3})\s", l)]
def cell(t):
    # escape angle brackets outside code spans only; inside one they print as typed
    parts = t.replace("|", "\\|").replace("\n", " ").split("`")
    return "`".join(p if i % 2 else p.replace("<", "&lt;").replace(">", "&gt;") for i, p in enumerate(parts))
rows = {"goatbuilder": [], "tools": []}
for c in codes:
    e = json.loads(subprocess.run([bin_, "--lang", "en", "explain", "-o", "json", c], capture_output=True, text=True, check=True).stdout)
    e = e.get("interpretation", e) if isinstance(e, dict) else e
    hit = e["hits"][0]["entry"] if "hits" in e else e
    row = "| `%s` | %s | %s | %s |" % (c, cell(hit.get("title", "")), cell(hit.get("what", "")), hit.get("exit", "") if c.startswith("GB-") else hit.get("tool", ""))
    rows["goatbuilder" if c.startswith("GB-") else "tools"].append(row)
text = open(dst, encoding="utf-8").read()
table = ["## goatbuilder's own codes", "", "| Code | Meaning | What happened | Exit |", "|---|---|---|---|", *rows["goatbuilder"],
         "", "## Tool errors goatbuilder interprets", "", "| Code | Meaning | What happened | Tool |", "|---|---|---|---|", *rows["tools"]]
open(dst, "w", encoding="utf-8").write(text.replace("<!-- error-table -->", "\n".join(table)))
PY

compose docs/operations/goatbuilder-first-run.md first-run.md
compose docs/design/goatbuilder.md goatbuilder.md
compose docs/design/goatbuilder-conformance.md goatbuilder-conformance.md
compose docs/design/goatbuilder-distribution.md goatbuilder-distribution.md
compose docs/tutorials/01-new-goatnet.md tutorial-1-new-goatnet.md
compose docs/tutorials/02-first-federated-node.md tutorial-2-first-federated-node.md
compose docs/design/goatnet-tutorials.md goatnet-tutorials.md

# Appendix D: every verb's --help, the verb list read from --help itself so a
# new verb is carried without editing this file. Help is rendered in English.
{
  echo "# Appendix D. Command reference"
  echo
  echo "Generated from \`goatbuilder --help\` of the binary this book was built with: $("$BIN" --version)."
  echo
  echo '```text'
  "$BIN" --lang en --help
  echo '```'
  for verb in $("$BIN" --lang en --help | awk '/^Commands:/{f=1;next} /^$/{f=0} f && /^  [a-z]/{print $1}' | grep -vx help); do
    echo
    echo "## goatbuilder $verb"
    echo
    echo '```text'
    "$BIN" --lang en "$verb" --help
    echo '```'
  done
} > "$OUT/reference/appendix-d-commands.md"

echo "composed: SUMMARY.md $(ls "$OUT" | tr '\n' ' ')$(ls "$OUT/reference" | wc -l | tr -d ' ') reference files"
