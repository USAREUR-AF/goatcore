# goatbuilder changelog entries

Each file here is one entry of goatbuilder's next release. A pull request
that changes what someone deploying or contributing sees adds one file here.
It does not edit `../CHANGELOG.md`, so two pull requests never conflict over
the changelog.

Name the file `<slug>.md`, in lower case letters, digits and hyphens, for
example `federate-phase-11-resume-restarts-link.md`. Start it with this
header:

```markdown
---
heading: For someone deploying
title: Fixed: a federate run killed in phase 11 resumes and brings the link up
---

What changed, for the reader under that heading, in Markdown. End with the
pull request's number, for example (#597).
```

- `heading` is one of "For someone deploying", "For someone contributing"
  or "To be removed".
- `title` is one line, usually starting with "Added:", "Changed:",
  "Fixed:" or "Deprecated:". Write it as plain words after `title:`: the
  colon after "Added" needs no quotes. A title wrapped in one pair of
  quotes is read, and rendered, without them.
- The body has no heading of level 1, 2 or 3; use `####` or deeper.

Then render the changelog's Unreleased section and commit both files:

```bash
python3 ops/release/changelog.py write
```

CI runs `python3 ops/release/changelog.py check`, which fails when an entry
is malformed or when the Unreleased section differs from its render. The
version pull request of a release runs `changelog.py release <version>
<date>`, which moves every entry here into the release's section and removes
the files. The design is in
[community-and-stable-line.md](../../../docs/design/community-and-stable-line.md),
"The changelog, one file per entry".
