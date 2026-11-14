---
heading: For someone deploying
title: "Added: join-node builds a node on a phone with --class phone --via usb"
---

`goatbuilder join-node <environment> --site <site> --node <node> --class phone
--via usb` joins the Linux VM on an Android phone as a `core` joiner
(docs/design/phone-node.md). Built on a fixture; Unmeasured on a phone.

- `--via usb` reaches the node over the Android debug bridge until it is
  enrolled. The run writes the SSH configuration and the Ansible inventory
  line it uses into the state directory (`ops/scripts/phone-node-ssh.sh`), so
  none is written by hand, and takes no `--address`. From phase 12 the node is
  reached at its mesh address. A phone that does not answer is refused
  (GB-299).
- For `--class phone`, phase 2 prepares the phone's VM (Debian's kernel with
  its boot settings, Docker Engine, the deploy user) and then runs the
  integrity checks in it (`ops/scripts/phone-node-provision.sh`). A check that
  fails stops the run (GB-300). A VM already prepared goes straight to the
  checks. The script also runs by itself.
- The machine class `phone` declares once what a phone node differs in
  (`ops/ansible/group_vars/class_phone.yml`): no LAN address, no public
  ingress, no disk encryption inside the VM, memory limits for 4 GB, no
  observability host. The site lists its node under `[class_phone]` in
  `inventory/mgmt-hosts.ini` and writes none of those values; the config
  preflight takes `--class phone` and refuses a site that contradicts them
  (GB-301).

Every other class and transport plans and runs as before. (#747)
