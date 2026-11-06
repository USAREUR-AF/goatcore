# Fixtures for check-peering-forwarder-holds-nothing.py

Each directory is a forwarder role (and, where named, a playbook) with one way of
handing a forwarder peering material. `--self-test` runs the guard on each and fails
if any is accepted, then runs it on the real role and fails if that is refused.
