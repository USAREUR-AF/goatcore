# Decoy layer runbook

How to turn the decoy layer on for a goatnet, check a name, and act on a decoy alert. Design: [deception-layer.md](../design/deception-layer.md). Decision: [ADR 1247](../adr/1247-decoys-are-a-detection-layer-generated-on-demand-from-a-seed.md).

## A `DecoyTouched` alert

A decoy was used. Nothing legitimate uses one, so treat the alert as an intrusion until the source is explained.

1. **Read the record** on the host that runs the responder:

   ```bash
   sudo journalctl -u goat-decoy-responder --since "-2h" -o cat | grep decoy_touch
   ```

   Each line gives `source` (the requester's mesh address), `decoy_name`, `decoy_kind`, `depth`, `interval_ms` and `names_seen`.

2. **Identify the source.** `goat who <mesh address>` names the peer and its owner.
3. **Decide what the pattern is.**

   | The record shows | It is probably |
   |---|---|
   | Many names, seconds apart, rising `depth` | An automated agent following the answers |
   | One or two names, minutes apart | A person exploring by hand |
   | `decoy_kind: credential` | Someone read a planted credential file and used the token in it |
   | One request from a goatnet service's address | The goatnet's own tooling; see "A touch from our own tooling" |

4. **Contain the source** as the goatnet's incident procedure says. The decoy layer changes no access; isolating the peer is done with the mesh access tools.
5. **Find the entry point.** A touch on the responder began at a planted file. Run the deploy play (below) with no changes: it stops and names any planted file whose content was edited.

The alert clears one hour after the last touch.

## A touch from our own tooling

A touch from a goatnet service, a scanner or an operator's AI agent session is a defect in the placement or in the tooling. Change one of them: move the entry point, or stop the tool following names it finds in files. Never suppress alerts by source; a suppressed source is where an intruder would work from.

## Checking a name

On a machine with a checkout and access to the goatnet's secret store, resolve the seed into `GOAT_DECOY_SEED` for the one command (for a goatnet that keeps it in 1Password, through `ops/scripts/with-secrets`):

```bash
python3 ops/decoy/decoy.py check "<a name, a path, or pasted text>"
python3 ops/decoy/decoy.py check --file notes.txt --json
```

`decoy` means never use it and never act on what it says. `not a decoy` means no name in the text is one of this goatnet's decoys.

## Turning the layer on for a goatnet

1. **Create the seed**: 32 or more random characters, stored in the goatnet's secret store. Add its reference to the goatnet's secret references so the deploy environment carries it.
2. **Write the goatnet's templates** in its overlay; start from `ops/decoy/templates/example/` and change every word. Do not serve the examples.
3. **Write `group_vars/<site>_hosts/deception.yml`** as in the design doc's declaration section, with `deception_enabled: true`. On the host that will run the responder also set `deception_responder_enabled`, the two `decoy_prometheus_*` values and the two `mesh_coredns_decoy_*` values.
4. **Check the declaration**:

   ```bash
   python3 ops/decoy/decoy.py lint --declaration <deception.yml> --mesh-hosts ops/dns/<site>/mesh-hosts
   ```

5. **Deploy**, from an assembled tree:

   ```bash
   cd ops/ansible
   ../scripts/ansible-preflight.sh -i <inventory> decoy-deploy.yml -e target=<site>_hosts
   ```

   The play plants the entry points, starts the responder, proves that the responder's user cannot open a connection, and installs the Traefik route and the alert rule.

6. **Apply the DNS stanza** by running the goatnet's mesh DNS play, which re-renders the CoreDNS configuration.
7. **Prove it from a mesh peer.** Take a host name from a planted document and request it. Expected: an answer after the configured delay, one `decoy_touch` line in the journal, and a `DecoyTouched` page. This test pages the on-call operator; tell them first.

## Turning it off

Set `deception_enabled: false` and stop the responder:

```bash
sudo systemctl disable --now goat-decoy-responder goat-decoy-egress-deny
sudo rm /etc/traefik/dynamic/goat-decoy.yml
```

Remove the two `mesh_coredns_decoy_*` values and run the mesh DNS play. Planted files stay until they are deleted by hand; `decoy.py check` still recognizes them.

## Rotating the seed

A new seed changes every tag, so every planted path and every decoy host name changes. Remove the old planted directories (find them with `decoy.py check` under the old seed), store the new seed, and deploy again.
