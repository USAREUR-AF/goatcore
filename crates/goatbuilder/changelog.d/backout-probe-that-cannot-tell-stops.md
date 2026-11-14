---
heading: For someone deploying
title: Fixed: a back-out stops on a probe that cannot tell, and takes back what admission placed
---

`goatbuilder federate backout` reads a thing as absent only on the exact
"not found" line its tool prints for that name (docker, podman, `wg`, `ip`,
`nft` and SPIRE). A probe that failed any other way, such as a folder it
could not list or a system bus it could not reach, reads `unknown`, and an
`unknown` row stops the back-out instead of counting as absent.
**Behaviour change:** a back-out that used to pass over such a probe now
stops on it.

The back-out also takes back what admission placed, by the route admission
took. A core fragment the driver placed (`FED_CORE_*`) is removed the same
way. On a core deployed by the goat-zenoh role, the core router's
certificate and key are deleted only where they are byte for byte this
peering's leaf, and our core router's leaf is restored from the earliest
admission that touched it. An IPsec link's MSS clamp rules are removed by
their own handles (#554).
