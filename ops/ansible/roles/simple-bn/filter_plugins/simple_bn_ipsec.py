# SPDX-License-Identifier: Apache-2.0
"""simple_bn_ipsec - read a simple-bn's encryption stage 2 IPsec declaration (docs/design/simple-boundary-node.md §4).

    {{ simple_bn | simple_bn_ipsec(env) }}

`env` holds the values of the environment variables the declaration names, read by the role
with lookup('env', ...) in a no_log task: gateway, our_public, psk, remote_ts, dest.

Returns {"wanted": bool, "errors": [str], "conf": {...}}. Error strings name declaration keys
and environment variable names only, never a value, so the role can print them. `conf` is what
templates/swanctl-simple-bn.conf.j2 renders; it holds the PSK (base64), so the role keeps it
in no_log tasks. The filter never raises.
"""
import base64
import ipaddress
import re

DEFAULT_IKE = "aes256gcm16-prfsha384-ecp384"
DEFAULT_ESP = "aes256gcm16-ecp384"
DEFAULT_DPD = "30s"
# Responder timers: the initiator drives every rekey. We never start an IKE or CHILD rekey
# (rekey_time 0s, so no collision with the initiator's), and a CHILD_SA the initiator failed to
# rekey is closed at its hard life rather than kept: 75m = a 1 h ESP lifetime (a common gateway
# default) plus 15 minutes. A dead initiator's IKE_SA is removed by DPD (dpd_action clear).
RESPONDER_TIMES = {"ike_rekey_time": "0s", "child_rekey_time": "0s", "child_life_time": "75m"}
PSK_MIN = 16
INTERFACE = "sbn-ipsec"          # policy mode: a dummy interface holding our public address
XFRM_INTERFACE = "sbn-xfrm"      # route mode: the XFRM interface the tunnel is routed through
DEFAULT_IF_ID = 42
ROUTE_TABLE = 4101               # route mode: sends from the tunnel address look up only this table

_PROPOSALS = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*(,[a-z0-9]+(-[a-z0-9]+)*)*$")
_TIME = re.compile(r"^[0-9]+[smhd]?$")
_HOST = re.compile(r"^[A-Za-z0-9]([A-Za-z0-9.-]{0,251}[A-Za-z0-9])?$")
_FILE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_ID = re.compile(r'^[^"\n\r{}#]{1,255}$')


def _ipv4(value):
    try:
        return str(ipaddress.IPv4Address(value))
    except (ipaddress.AddressValueError, ValueError):
        return None


def _networks(value):
    """Comma-separated IPv4 addresses or CIDRs -> list of networks, or None if any is malformed."""
    out = []
    for part in [p.strip() for p in str(value).split(",") if p.strip()]:
        try:
            out.append(ipaddress.IPv4Network(part if "/" in part else part + "/32", strict=True))
        except (ipaddress.AddressValueError, ipaddress.NetmaskValueError, ValueError):
            return None
    return out or None


def simple_bn_ipsec(simple_bn, env=None):
    env = env or {}
    sbn = simple_bn if isinstance(simple_bn, dict) else {}
    enc = sbn.get("encryption") or {}
    ip = enc.get("ipsec") if isinstance(enc, dict) else None
    try:
        stage = int(enc.get("stage", 0)) if isinstance(enc, dict) else 0
    except (TypeError, ValueError):
        stage = -1
    if ip is None:
        return {"wanted": False, "errors": [], "conf": {}}
    errors = []
    if stage != 2:
        errors.append("encryption.ipsec is declared but encryption.stage is not 2")
    if not isinstance(ip, dict):
        return {"wanted": True, "errors": errors + ["encryption.ipsec must be a mapping"], "conf": {}}

    receiver = sbn.get("receiver") or {}
    label = str(receiver.get("label", ""))
    conf = {"name": "simple-bn-" + label, "interface": INTERFACE}

    # Initiator (default: we dial the gateway) or responder (the receiver dials us, from an address
    # we may not know: a gateway behind NAT). `initiator: false` and `role: responder` are the same.
    flag, role = ip.get("initiator"), ip.get("role")
    if role is None:
        role = "responder" if flag is False else "initiator"
    if role not in ("initiator", "responder") or flag not in (None, True, False) or (
            flag is not None and flag != (role == "initiator")):
        errors.append("encryption.ipsec.role must be initiator or responder, and agree with initiator: true|false")
    initiator = role != "responder"
    conf["initiator"] = initiator

    # The receiver's gateway (IKE peer): an address or a name, from the environment. A responder
    # may leave it out (remote_addrs %any) and then admits only remote_id with the key.
    gw_env = ip.get("gateway_env")
    gateway = str(env.get("gateway") or "").strip()
    if not gw_env and initiator:
        errors.append("encryption.ipsec.gateway_env is required (the env var holding the receiver's gateway address)")
    elif not gw_env:
        pass
    elif not gateway:
        errors.append(f"{gw_env} (encryption.ipsec.gateway_env) is not set in the environment")
    elif not (_ipv4(gateway) or _HOST.match(gateway)):
        errors.append(f"{gw_env} (encryption.ipsec.gateway_env) is not an IPv4 address or a host name")
    conf["gateway"] = gateway
    conf["remote_addrs"] = gateway or "%any"

    # Policy mode (default): our public /32 to the receiver's inside address is the selector, and
    # the kernel's IPsec policy picks the traffic. Route mode: an XFRM interface; what is routed
    # into it is encrypted, and the selectors default to 0.0.0.0/0 (what a route-based gateway with
    # no proxy IDs proposes).
    mode = ip.get("mode", "policy")
    if mode not in ("policy", "route"):
        errors.append("encryption.ipsec.mode must be policy or route")
    route = mode == "route"
    conf["mode"] = mode

    # Our static public address: the default IKE identity; in policy mode also the local selector
    # and the sender's source. Route mode needs it only when local_id is not declared.
    our_env = ip.get("our_public_env") or receiver.get("our_source_env")
    our = _ipv4(str(env.get("our_public") or "").strip())
    if not (route and ip.get("local_id")):
        if not our_env:
            errors.append("encryption.ipsec.our_public_env (or receiver.our_source_env) is required: our static public IPv4 address")
        elif not our:
            errors.append(f"{our_env} (our public address) is not set in the environment or is not an IPv4 address")
    conf["our_public"] = our or ""

    # The receiver's inside address: the host of receiver.dest_env, where the sender sends.
    dest = str(env.get("dest") or "").strip()
    dest_host = _ipv4(dest.rsplit(":", 1)[0]) if ":" in dest else None

    if route:
        conf["interface"] = XFRM_INTERFACE
        try:
            conf["if_id"] = int(ip.get("if_id", DEFAULT_IF_ID))
            if not 1 <= conf["if_id"] <= 0xFFFFFFFE:
                raise ValueError
        except (TypeError, ValueError):
            errors.append("encryption.ipsec.if_id must be an integer from 1 to 4294967294")
            conf["if_id"] = DEFAULT_IF_ID
        conf["table"] = ROUTE_TABLE

        def given(key):
            """A tunnel value, literal (key) or from the env var key_env names."""
            if ip.get(key) is not None:
                return str(ip[key]).strip(), f"encryption.ipsec.{key}"
            if ip.get(key + "_env"):
                return str(env.get(key) or "").strip(), f"{ip[key + '_env']} (encryption.ipsec.{key}_env)"
            return "", None

        cidr, cidr_src = given("tunnel_cidr")
        local, local_src = given("tunnel_local")
        remote, remote_src = given("tunnel_remote")
        net = None
        if not cidr_src:
            errors.append("encryption.ipsec.tunnel_cidr (or tunnel_cidr_env) is required in route mode (the point-to-point tunnel subnet)")
        else:
            try:
                net = ipaddress.IPv4Network(cidr, strict=True)
                if net.prefixlen > 31 or net.prefixlen < 24:
                    raise ValueError
            except (ipaddress.AddressValueError, ipaddress.NetmaskValueError, ValueError):
                errors.append(f"{cidr_src} is not an IPv4 network from /24 to /31")
                net = None
        hosts = {}
        for key, val, src in (("tunnel_local", local, local_src), ("tunnel_remote", remote, remote_src)):
            if not src:
                errors.append(f"encryption.ipsec.{key} (or {key}_env) is required in route mode")
                continue
            addr = _ipv4(val)
            if not addr:
                errors.append(f"{src} is not set or is not an IPv4 address")
            elif net is not None and (ipaddress.IPv4Address(addr) not in net or (
                    net.prefixlen < 31 and ipaddress.IPv4Address(addr) in (net.network_address, net.broadcast_address))):
                errors.append(f"{src} is not a host address inside tunnel_cidr")
            else:
                hosts[key] = addr
        if len(hosts) == 2 and hosts["tunnel_local"] == hosts["tunnel_remote"]:
            errors.append("encryption.ipsec tunnel_local and tunnel_remote are the same address")
        conf["tunnel_local"] = f"{hosts.get('tunnel_local', '')}/{net.prefixlen if net else 32}"
        conf["tunnel_remote"] = hosts.get("tunnel_remote", "")
        conf["src_addr"] = hosts.get("tunnel_local", "")
        conf["tunnel_net"] = str(net) if net is not None else ""
        if not dest_host:
            errors.append("route mode needs receiver.dest_env to hold an IPv4 HOST:PORT (the receiver's inside address, routed into the tunnel)")
        conf["routes"] = [f"{dest_host}/32"] if dest_host else []
        if dest_host and net is not None and ipaddress.IPv4Address(dest_host) in net:
            conf["routes"] = []   # on the tunnel subnet itself: the interface's own address route carries it
        # Selectors: 0.0.0.0/0 both sides unless declared.
        sel = {}
        for key in ("local_ts", "remote_ts"):
            raw = ip.get(key)
            if raw is None and key == "remote_ts" and ip.get("remote_ts_env"):
                raw = env.get("remote_ts") or ""
            nets = _networks(",".join(raw) if isinstance(raw, list) else raw) if raw is not None else [ipaddress.IPv4Network("0.0.0.0/0")]
            if not nets:
                errors.append(f"encryption.ipsec.{key} is not a list of IPv4 addresses or CIDRs")
            sel[key] = [str(n) for n in nets] if nets else []
        conf["local_ts"] = ",".join(sel["local_ts"])
        conf["remote_ts"] = ",".join(sel["remote_ts"])
        conf["remote_ts_list"] = sel["remote_ts"]
    else:
        conf["src_addr"] = our or ""
        conf["local_ts"] = f"{our}/32" if our else ""
        # The receiver's inside address(es): declared env var, else the receiver address (dest_env) itself.
        rts_env = ip.get("remote_ts_env")
        if rts_env:
            nets = _networks(env.get("remote_ts") or "")
            if not nets:
                errors.append(f"{rts_env} (encryption.ipsec.remote_ts_env) is not set or is not a comma-separated list of IPv4 addresses or CIDRs")
        else:
            nets = _networks(dest_host) if dest_host else None
            if not nets:
                errors.append("encryption.ipsec.remote_ts_env is required when receiver.dest_env does not hold an IPv4 HOST:PORT "
                              "(the receiver's inside address is the remote traffic selector)")
        conf["remote_ts"] = ",".join(str(n) for n in nets) if nets else ""
        conf["remote_ts_list"] = [str(n) for n in nets] if nets else []
        gw_ip = _ipv4(gateway)
        if nets and gw_ip and any(ipaddress.IPv4Address(gw_ip) in n for n in nets):
            errors.append("the receiver's gateway address lies inside the remote traffic selector; the tunnel carries traffic to an "
                          "inside address behind the gateway, never to the gateway itself")

    # Identities: default to the two public addresses; a receiver may name others (FQDN, e-mail, key-id
    # or a DN). A responder must name the initiator's: with remote_addrs %any it is what admits them.
    if not initiator and not ip.get("remote_id"):
        errors.append("encryption.ipsec.remote_id is required for a responder (the receiver gateway's own identification, agreed with them)")
    for key, default in (("local_id", our or ""), ("remote_id", gateway)):
        val = str(ip.get(key) or default)
        if ip.get(key) is not None and (not _ID.match(val) or val.startswith("%")):
            errors.append(f"encryption.ipsec.{key} holds a character swanctl cannot take (quote, brace, # or newline) or is a % wildcard")
        conf[key] = val

    # Authentication.
    auth = ip.get("auth")
    conf["auth"] = auth
    if auth == "psk":
        psk_env = ip.get("psk_env")
        psk = env.get("psk") or ""
        if not psk_env:
            errors.append("encryption.ipsec.psk_env is required with auth psk (the env var holding the pre-shared key)")
        elif not psk:
            errors.append(f"{psk_env} (encryption.ipsec.psk_env) is not set in the environment")
        elif len(psk) < PSK_MIN:
            errors.append(f"{psk_env} (encryption.ipsec.psk_env) is shorter than {PSK_MIN} characters")
        conf["psk_b64"] = base64.b64encode(psk.encode()).decode() if psk else ""
    elif auth == "pubkey":
        pk = ip.get("pubkey") or {}
        if not isinstance(pk, dict):
            pk = {}
        for key in ("cert", "key"):
            if not pk.get(key):
                errors.append(f"encryption.ipsec.pubkey.{key} is required with auth pubkey")
            elif not _FILE.match(str(pk[key])):
                errors.append(f"encryption.ipsec.pubkey.{key} must be a plain file name")
        has_ca, has_cert = bool(pk.get("remote_cacert")), bool(pk.get("remote_cert"))
        if has_ca == has_cert:
            errors.append("encryption.ipsec.pubkey needs exactly one of remote_cacert (the receiver's CA) or remote_cert (their certificate)")
        for key in ("remote_cacert", "remote_cert"):
            if pk.get(key) and not _FILE.match(str(pk[key])):
                errors.append(f"encryption.ipsec.pubkey.{key} must be a plain file name")
        conf.update({k: str(pk.get(k) or "") for k in ("cert", "key", "remote_cacert", "remote_cert")})
    else:
        errors.append("encryption.ipsec.auth must be psk or pubkey")

    # Proposals and timers.
    for key, default in (("ike_proposals", DEFAULT_IKE), ("esp_proposals", DEFAULT_ESP)):
        val = str(ip.get(key) or default)
        if not _PROPOSALS.match(val):
            errors.append(f"encryption.ipsec.{key} is not a strongSwan proposal list (e.g. {default})")
        conf[key] = val
    times = {"ike_rekey_time": "", "child_rekey_time": "", "child_life_time": ""} if initiator else RESPONDER_TIMES
    for key, default in [("dpd_delay", DEFAULT_DPD)] + list(times.items()):
        val = str(ip[key] if ip.get(key) is not None else default)
        if val and not _TIME.match(val):
            errors.append(f"encryption.ipsec.{key} is not a time (a number with s, m, h or d)")
        conf[key] = val

    return {"wanted": True, "errors": errors, "conf": conf}


class FilterModule:
    def filters(self):
        return {"simple_bn_ipsec": simple_bn_ipsec}
