---
heading: For someone deploying
title: "Added: a source may be pinned by a signed tag and the key that signs it"
---

A component's or a target type's `source:` may name `tag` and `signer` (the
SSH public key that signs the tag) in place of `commit`. Every run fetches the
tag, takes it only when its SSH signature is that key's, and uses the commit
it names now. A missing tag, a lightweight or unsigned tag, another key's
signature, or another tag under its name is refused before any host or phase
(GB-385, GB-420); a tag with no signer is refused when the file is read.
