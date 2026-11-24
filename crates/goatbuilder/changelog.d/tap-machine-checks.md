---
heading: For someone deploying
title: "Added: `doctor --target network-tap --live` checks a tap's end state on the machine; `status` prints its mode"
---

`goatbuilder doctor --target network-tap --site <site> --live` reads each tap
over Ansible, changes nothing, and holds it to its declared mode: no address on
a tap port or the bridge, nothing that sends, management and the default route
on the third interface, the bridge (inline) or the dropped egress (listen),
receive offloads and Energy-Efficient Ethernet off, the recorder on a port, no
port that boot waits for, a hardware watchdog, and an encrypted root. A fact that
cannot be read is unknown, which fails. Without `--live` no machine is reached.

`goatbuilder status` prints each tap's mode, `inline, fail-closed` or `listen,
fail-open`, from the checkout.
