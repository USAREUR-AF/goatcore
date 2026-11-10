# Local registry: goat images from a registry you hold

[pluggable-dependencies.md](../../../docs/design/pluggable-dependencies.md) row 13. A goatnet runs goat's images from a registry the team holds — on the box, on a laptop on the LAN, or the team's own — instead of `ghcr.io/usareur-af`. Every image keeps its name and its pinned digest, so what runs is byte for byte what the pin names.

## On a goatlite

1. Start a registry. On the box itself (Docker reaches `127.0.0.1` over plain HTTP without configuration):

   ```bash
   ops/recipes/local-registry/local-registry.sh up --addr 127.0.0.1:5000
   ```

   Or on a laptop on the box's LAN, which then serves as the image cache for every box you stand up there:

   ```bash
   ops/recipes/local-registry/local-registry.sh up --addr 0.0.0.0:5000
   ```

2. Copy every goat image a goatlite runs into it (needs `crane` or `skopeo` where you run it; `--insecure` for a registry without TLS):

   ```bash
   ops/recipes/local-registry/local-registry.sh mirror --to 127.0.0.1:5000/usareur-af --insecure
   ```

   Each line reads `ok <registry>/<name>@<digest>`; an image the registry does not serve at its pinned digest fails the run by name. `verify` re-checks later; `list` prints source and destination without copying.

3. Add one line to `goatlite.yaml` and render again (`goatbuilder standup` renders first):

   ```yaml
   lite:
     image_registry: 127.0.0.1:5000/usareur-af    # or <laptop-address>:5000/usareur-af
   ```

   The renderer pins every goat image of the goatnet at that registry in `ops/pins/lite-pins.yaml`. For a registry that is not on the box, it also sets `docker_engine_insecure_registries` in the goatnet's `vars.yml`, which phase 2 writes into the box's `/etc/docker/daemon.json`; a registry with a certificate the box trusts is reached over TLS either way.

Images that are not goat's keep their own registries (`ghcr.io/spiffe/spire-*`, `docker.io/library/nginx`), and so do images a role names outside the pin table.

## On another goatnet

`local-registry.sh mirror --member <site>_hosts --to <registry>/<namespace>` copies that goatnet's goat images; `list --member <site>_hosts --to …` prints each destination reference. Those references become the goatnet's pins as deviations in its pin table, signed with its own CA (`ops/pins/generate-pins.py`, environment-parity.md §3). That step is a pin change in the team's own table, not automated here.

## Test

`ops/recipes/local-registry/test.sh` brings a registry up (Docker's `registry:3`, or crane's built-in registry where Docker is not running), mirrors two goat images, refuses an image it did not mirror, checks that the pins a goatlite with `image_registry` renders name exactly what the registry serves, and with Docker pulls a rendered pin from it at its digest. CI: the `local-registry` job of `.github/workflows/zero-saas-recipes.yml`.
