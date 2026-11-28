---
heading: For someone deploying
title: Added: doctor --target --live checks a small computer against what its model delivers
---

`goatbuilder doctor --target <type> --site <site> --live` now reads, as root over
Ansible, each machine that declares `platform_family: small-computer`, and holds
it to its model's `delivers:` entry in `ops/bootstrap/hardware-classes.yaml`
(today `rpi5`). One row each: the Ubuntu version and the board; the root on
LUKS2 on NVMe, with FIPS-approved primitives and no unfinished reencryption;
every way the root unlocks at boot; FIPS mode; SSH key-only with approved
algorithms; first boot finished. The collector,
`ops/scripts/small-computer-endstate.py`, reads and changes nothing, and prints
no salt, digest, key or address. A fact it cannot read is unknown, never a
pass. A model entry that cannot be read is GB-410 (#1331).
