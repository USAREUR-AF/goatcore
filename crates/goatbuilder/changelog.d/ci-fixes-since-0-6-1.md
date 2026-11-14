---
heading: For someone contributing
title: Fixed: CI runs on main to the end, and several checks say why they fail
---

- A run on `main` is no longer cancelled by the next push to `main`, so the
  commit a release is tagged on keeps its own complete run (#618).
- The playbook syntax step prints each failing playbook's error, and an
  Ansible collection that a timed-out install left half written is
  installed again (#625).
- `.envrc.example` is regenerated, and a check holds it to the environment
  variables the repository reads (#627).
- The filter plugin tests moved out of `ops/ansible/filter_plugins`, so
  Ansible no longer imports them on every playbook run (#626).
- The hermetic deploy job's release re-run and soak cases pass on a run
  that crosses midnight UTC (#621).
