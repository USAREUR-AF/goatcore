# SMTP sink: invite mail with no mail account

[pluggable-dependencies.md](../../../docs/design/pluggable-dependencies.md) row 14. The identity console mails each candidate a one-time link through SES. With `id_admin_console_smtp_host: ""` (the role's default) mail is off and the console shows the link for the administrator to hand over. The sink is the second option: the console sends as usual, and Mailpit on the goatnet's own docker network keeps every message instead of delivering it. The administrator reads the invite there and passes the link on.

## Turn it on for a goatnet

In `ops/ansible/group_vars/<site>_hosts/vars.yml`:

```yaml
smtp_sink_enabled: true
id_admin_console_smtp_host: "goat-smtp-sink"
id_admin_console_smtp_port: "1025"
id_admin_console_smtp_from: "onboard@<zone>"
id_admin_console_smtp_reply_to: "<the administrator's address>"
```

`id_admin_console_smtp_username` and `_password` stay empty: an empty username sends without SMTP AUTH, which is what the sink takes. Then run the sink's play; the console reads the new settings on its next deploy:

```bash
cd ops/ansible
../scripts/ansible-preflight.sh -i inventory/mgmt-hosts.ini smtp-sink.yml -e target=<site>_hosts
```

Read the captured mail through an SSH tunnel to the box (the UI listens on the box's loopback only):

```bash
ssh -L 8025:127.0.0.1:8025 <ssh_user>@<box>
```

then open `http://127.0.0.1:8025`.

| Setting | Default | Meaning |
|---|---|---|
| `smtp_sink_network` | `mgmt` | the docker network the console is on |
| `smtp_sink_ui_publish` | `127.0.0.1:8025` | web UI and API on the host |
| `smtp_sink_smtp_publish` | `""` | SMTP on the host as well (off: only the docker network) |
| `smtp_sink_max_messages` | `500` | messages kept before the oldest is dropped |

## Where it takes effect

The console sends without AUTH when its username is empty from the console build that carries `internal/mailer` `auth()`; a goatnet gets that once a console image built from it is pinned in `fleet-pins.yaml`. A goatlite does not run the identity console today (`LITE_ABSENT` in `ops/scripts/goatlite-render.py`).

## Test

`ops/recipes/smtp-sink/test.sh` (Linux, Docker, passwordless sudo): the play deploys nothing on a site without `smtp_sink_enabled`; with it, Mailpit comes up on the declared network; the console's own mailer sends an invite with an empty username and the test reads it back, link included, from the sink's API; a username against the same sink is refused; a second play run changes nothing. CI: the `smtp-sink` job of `.github/workflows/zero-saas-recipes.yml`.
