#!/usr/bin/env python3
"""certbot manual hook: answer a DNS-01 challenge in Azure DNS.

Installed on Azure relays as /usr/local/sbin/goat-certbot-azure-dns01 by
ops/ansible/tasks/relay-dns01-azure.yml, and used as

  certbot certonly --manual --preferred-challenges dns \
    --manual-auth-hook    '/usr/local/sbin/goat-certbot-azure-dns01 auth' \
    --manual-cleanup-hook '/usr/local/sbin/goat-certbot-azure-dns01 cleanup'

certbot records both hooks in the renewal conf, so the systemd timer renews
unattended with the same code.

Auth is the relay VM's managed identity, through the instance metadata
service (IMDS). No credential is written to the relay and none comes from the
operator: the identity needs DNS Zone Contributor on the zone, granted in
Terraform by an admin. `check` proves that before certbot runs.

Config: a JSON file (default /etc/letsencrypt/azure-dns01.json) with
  zone_name, resource_group, subscription_id (empty: read from IMDS),
  auth ("managed_identity" | "user_assigned_identity"), client_id,
  ttl, propagation_seconds.

Python standard library only, so it adds no package to the relay.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

CONFIG_PATH = os.environ.get("GOAT_AZURE_DNS01_CONFIG", "/etc/letsencrypt/azure-dns01.json")
IMDS = "http://169.254.169.254/metadata"
ARM = "https://management.azure.com"
DNS_API = "2018-05-01"
AUTH_MODES = ("managed_identity", "user_assigned_identity")
CHALLENGE_LABEL = "_acme-challenge"


class HookError(Exception):
    pass


def relative_name(domain, zone):
    """Record-set name of the challenge for `domain`, relative to `zone`."""
    domain = domain.rstrip(".").lower()
    zone = zone.rstrip(".").lower()
    if domain == zone:
        return CHALLENGE_LABEL
    if not domain.endswith("." + zone):
        raise HookError("domain %s is not in zone %s" % (domain, zone))
    return "%s.%s" % (CHALLENGE_LABEL, domain[: -(len(zone) + 1)])


def add_value(records, value):
    """TXT records with `value` added once, keeping any other values."""
    out = [r for r in records if r.get("value")]
    if not any(value in r["value"] for r in out):
        out.append({"value": [value]})
    return out


def remove_value(records, value):
    """TXT records with every record carrying `value` removed."""
    return [r for r in records if r.get("value") and value not in r["value"]]


def load_config(path):
    with open(path) as fh:
        cfg = json.load(fh)
    for key in ("zone_name", "resource_group"):
        if not cfg.get(key):
            raise HookError("%s: %s is empty" % (path, key))
    if cfg.get("auth", "managed_identity") not in AUTH_MODES:
        raise HookError("%s: auth must be one of %s" % (path, ", ".join(AUTH_MODES)))
    if cfg.get("auth") == "user_assigned_identity" and not cfg.get("client_id"):
        raise HookError("%s: user_assigned_identity needs client_id" % path)
    return cfg


# IMDS must never go through a proxy.
_imds_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def _imds_get(path, query):
    url = "%s/%s?%s" % (IMDS, path, urllib.parse.urlencode(query))
    req = urllib.request.Request(url, headers={"Metadata": "true"})
    try:
        with _imds_opener.open(req, timeout=10) as resp:
            return resp.read().decode()
    except urllib.error.HTTPError as err:
        raise HookError("IMDS %s: HTTP %s (has the VM a managed identity?)" % (path, err.code))
    except urllib.error.URLError as err:
        raise HookError("IMDS %s unreachable: %s (is this an Azure VM?)" % (path, err.reason))


def imds_token(cfg):
    query = {"api-version": "2018-02-01", "resource": ARM + "/"}
    if cfg.get("auth") == "user_assigned_identity":
        query["client_id"] = cfg["client_id"]
    return json.loads(_imds_get("identity/oauth2/token", query))["access_token"]


def subscription_id(cfg):
    if cfg.get("subscription_id"):
        return cfg["subscription_id"]
    return _imds_get(
        "instance/compute/subscriptionId", {"api-version": "2021-02-01", "format": "text"}
    ).strip()


def _arm(method, url, token, body=None, headers=None):
    data = json.dumps(body).encode() if body is not None else None
    h = {"Authorization": "Bearer " + token, "Content-Type": "application/json"}
    h.update(headers or {})
    req = urllib.request.Request(url, data=data, headers=h, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as err:
        return err.code, {}


def _zone_url(cfg):
    return "%s/subscriptions/%s/resourceGroups/%s/providers/Microsoft.Network/dnsZones/%s" % (
        ARM,
        urllib.parse.quote(subscription_id(cfg)),
        urllib.parse.quote(cfg["resource_group"]),
        urllib.parse.quote(cfg["zone_name"]),
    )


def _txt_url(cfg, name):
    return "%s/TXT/%s?api-version=%s" % (_zone_url(cfg), urllib.parse.quote(name), DNS_API)


def _update(cfg, token, name, change):
    """Read-modify-write one TXT record set, guarded by its ETag."""
    url = _txt_url(cfg, name)
    for _ in range(5):
        status, current = _arm("GET", url, token)
        if status == 404:
            records, cond = [], {"If-None-Match": "*"}
        elif status == 200:
            records = current.get("properties", {}).get("TXTRecords", [])
            cond = {"If-Match": current.get("etag", "*")}
        else:
            raise HookError("GET %s: HTTP %s" % (name, status))
        new = change(records)
        if new == records and status == 404:
            return
        if new:
            body = {"properties": {"TTL": int(cfg.get("ttl", 60)), "TXTRecords": new}}
            status, _ = _arm("PUT", url, token, body, cond)
        else:
            status, _ = _arm("DELETE", url, token, None, cond)
        if status in (200, 201, 202, 204) or (not new and status == 404):
            return
        if status != 412:
            raise HookError("write %s: HTTP %s (DNS Zone Contributor granted?)" % (name, status))
    raise HookError("write %s: record kept changing underneath (HTTP 412 five times)" % name)


def cmd_check(cfg):
    token = imds_token(cfg)
    status, _ = _arm("GET", "%s?api-version=%s" % (_zone_url(cfg), DNS_API), token)
    if status != 200:
        raise HookError(
            "zone %s in %s: HTTP %s. The VM identity needs DNS Zone Contributor on the zone."
            % (cfg["zone_name"], cfg["resource_group"], status)
        )
    print("ok: managed identity can read zone %s" % cfg["zone_name"])


def cmd_auth(cfg):
    domain, value = os.environ["CERTBOT_DOMAIN"], os.environ["CERTBOT_VALIDATION"]
    name = relative_name(domain, cfg["zone_name"])
    _update(cfg, imds_token(cfg), name, lambda r: add_value(r, value))
    time.sleep(int(cfg.get("propagation_seconds", 30)))


def cmd_cleanup(cfg):
    domain, value = os.environ["CERTBOT_DOMAIN"], os.environ["CERTBOT_VALIDATION"]
    name = relative_name(domain, cfg["zone_name"])
    _update(cfg, imds_token(cfg), name, lambda r: remove_value(r, value))


def main(argv):
    commands = {"check": cmd_check, "auth": cmd_auth, "cleanup": cmd_cleanup}
    if len(argv) != 2 or argv[1] not in commands:
        sys.stderr.write("usage: %s check|auth|cleanup\n" % argv[0])
        return 64
    try:
        commands[argv[1]](load_config(CONFIG_PATH))
    except (HookError, KeyError, OSError, ValueError) as err:
        sys.stderr.write("goat-certbot-azure-dns01: %s\n" % err)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
