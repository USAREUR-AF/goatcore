"""sovereign_pki.py — Ansible filters over ops/identity/sovereign-pkis.yaml for the
icam-mtls-proxy role (ADR 1209). The logic lives in ops/identity/sovereign_pkis.py;
this file only exposes it to templates, so the role and the CI gate render the same
nginx maps.

  {{ pkis | sovereign_pki_nginx_maps }}       the map blocks (http context)
  {{ pkis | sovereign_pki_nginx_js_set }}     the js_set lines the maps need
  {{ pkis | sovereign_pki_nginx_profiles_js }} pki-profiles.js, or "" when no profile needs it
  {{ registry | sovereign_pki_select(ids) }}   the live profiles named in ids; raises on an
                                                unknown or sketch id (fail closed)

Run the test directly: python3 tests/ansible_filter_plugins/test_sovereign_pki.py
"""
import importlib.util
from pathlib import Path

_LIB = Path(__file__).resolve().parents[2] / "identity" / "sovereign_pkis.py"
_SPEC = importlib.util.spec_from_file_location("sovereign_pkis", _LIB)
_sp = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_sp)


def sovereign_pki_select(registry, ids):
    if not ids:
        raise ValueError("icam_mtls_proxy_pkis is empty: name at least one live profile from ops/identity/sovereign-pkis.yaml")
    chosen = []
    for i in ids:
        p = _sp.by_id(registry, i)
        if p.get("status") != "live":
            raise ValueError(f"PKI profile {i} is status={p.get('status')}; only a live profile may be deployed")
        chosen.append(p)
    return chosen


def sovereign_pki_nginx_maps(pkis):
    return _sp.nginx_maps(pkis)


def sovereign_pki_nginx_js_set(pkis):
    return "\n".join(_sp.nginx_js_set_lines(pkis))


def sovereign_pki_nginx_profiles_js(pkis):
    return _sp.nginx_profiles_js(pkis) or ""


class FilterModule:
    def filters(self):
        return {
            "sovereign_pki_select": sovereign_pki_select,
            "sovereign_pki_nginx_maps": sovereign_pki_nginx_maps,
            "sovereign_pki_nginx_js_set": sovereign_pki_nginx_js_set,
            "sovereign_pki_nginx_profiles_js": sovereign_pki_nginx_profiles_js,
        }
