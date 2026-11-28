#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Refuse an Alertmanager render that would drop a receiver the running config has.

A receiver's secret (a paging key, a heartbeat token, a webhook password) is
read from the deploy environment. When that environment lacks it, the template
renders the receiver without it, or leaves the receiver out, and the play
reports `changed`. The host then alerts nobody on that channel and nothing
says so.

Input, on stdin, one JSON object: {"live": "<yaml text>", "new": "<yaml text>"}.
`live` is the config the host runs now (empty on a first install), `new` is the
config about to be written.

Each config is reduced to a set of facts, one per notifier in each receiver:

    <receiver>/<kind>#<n>            the notifier exists
    <receiver>/<kind>#<n>/auth       it carries a credential

A fact in `live` and not in `new` is a loss. Any loss exits 1 and prints the
lost facts, which name receivers and kinds only: no address, no credential.

Exit 0 nothing lost, 1 a loss, 2 the input could not be read.
"""
import json
import sys

import yaml

KINDS = (
    "pagerduty_configs",
    "webhook_configs",
    "slack_configs",
    "email_configs",
    "opsgenie_configs",
    "victorops_configs",
    "pushover_configs",
    "sns_configs",
    "telegram_configs",
    "discord_configs",
    "msteams_configs",
    "webex_configs",
    "wechat_configs",
)
# Keys whose non-empty value is a credential, at the notifier's top level.
CRED_KEYS = (
    "routing_key", "routing_key_file", "service_key", "service_key_file",
    "api_key", "api_key_file", "api_url", "api_url_file",
    "auth_password", "auth_password_file", "auth_secret", "auth_identity",
    "token", "token_file", "user_key", "bot_token", "bot_token_file",
    "webhook_url", "webhook_url_file", "url_file",
)
HTTP_CRED_KEYS = ("authorization", "basic_auth", "oauth2", "bearer_token", "bearer_token_file")


def has_credential(notifier: dict) -> bool:
    for key in CRED_KEYS:
        if notifier.get(key):
            return True
    http = notifier.get("http_config")
    if isinstance(http, dict):
        for key in HTTP_CRED_KEYS:
            if http.get(key):
                return True
    return False


def facts(text: str) -> set:
    if not text or not text.strip():
        return set()
    doc = yaml.safe_load(text)
    if not isinstance(doc, dict):
        raise ValueError("config is not a mapping")
    out = set()
    for receiver in doc.get("receivers") or []:
        if not isinstance(receiver, dict):
            continue
        name = str(receiver.get("name", "?"))
        for kind in KINDS:
            for n, notifier in enumerate(receiver.get(kind) or []):
                if not isinstance(notifier, dict):
                    continue
                base = f"{name}/{kind}#{n}"
                out.add(base)
                if has_credential(notifier):
                    out.add(base + "/auth")
    return out


def main() -> int:
    try:
        payload = json.load(sys.stdin)
        live = facts(payload.get("live", ""))
        new = facts(payload.get("new", ""))
    except (ValueError, yaml.YAMLError) as exc:
        print(f"alertmanager-receiver-guard: unreadable input ({exc.__class__.__name__})")
        return 2
    lost = sorted(live - new)
    if lost:
        print("alertmanager-receiver-guard: the new config drops what the running one has:")
        for fact in lost:
            what = "the credential of " + fact[: -len("/auth")] if fact.endswith("/auth") else fact
            print(f"  - {what}")
        return 1
    print(f"alertmanager-receiver-guard: nothing lost ({len(new)} facts, {len(live)} before)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
