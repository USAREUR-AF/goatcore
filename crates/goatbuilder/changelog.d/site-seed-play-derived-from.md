---
heading: For someone deploying
title: Changed: a site's own seed play must say which generic play it was made from
---

A deployment may keep its own `ops/ansible/<site>-seed-from-peer.yml`, which
`goatbuilder standup` runs at phase 6 in place of the generic
`site-seed-from-peer.yml`. A copy made from an older generic play missed every
fix made to it since, and nothing said so. Such a copy now starts with
`# derived-from: site-seed-from-peer.yml sha256:<digest>`, the digest of the
generic play it was made from. Right before running it, standup checks that
digest against the current generic play (`ops/scripts/check-site-play-current.py`)
and refuses a copy with no such line, or made from an older version, with
GB-362. The message prints the digest to set once the copy is brought up to
date. A site with no copy, or whose file imports the generic play, is not
affected. An existing copy needs the line before its next standup run (#1129).
