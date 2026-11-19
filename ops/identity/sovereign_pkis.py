#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""sovereign_pkis.py — read, check and render ops/identity/sovereign-pkis.yaml.

One registry, every door (ADR 1209):

  nginx_maps(pkis)        the icam-mtls-proxy role's nginx `map` blocks: issuer DN
                          -> profile id, raw field -> identifier, -> X-Remote-User
                          "<id>:<identifier>" (ops/ansible/filter_plugins/sovereign_pki.py)
  nginx_profiles_js(pkis) per-profile njs wrappers, only for san_othername profiles
  terraform_value(pkis)   collab-services var.sovereign_pkis (cert-front)
  openbao_layout(pkis)    the store's OPENBAO_SOVEREIGN_PKI_DIR/<id>/ca-bundle.pem tree
  claim_map(pkis)         ops/identity/claim-maps/sovereign-pki.jsonnet (Kratos mapper)
  openssl_request(pki)    an openssl req config for a SYNTHETIC card in the PKI's shape
  identifier(pki, leaf)   the identity rule applied in Python (the reference the gate
                          and the tests hold the renderings to)

Needs PyYAML. No other third-party module.
"""
from __future__ import annotations

import pathlib
import re

import yaml

REGISTRY = pathlib.Path(__file__).resolve().parent / "sovereign-pkis.yaml"
SOURCES = ("san_upn", "san_email", "san_othername", "cn_regex", "serial")
STATUSES = ("live", "sketch")
UPN_OID = "1.3.6.1.4.1.311.20.2.3"
ID_RE = re.compile(r"^[a-z][a-z0-9-]{1,30}$")
NATION_RE = re.compile(r"^(?:[A-Z]{3}|NATO)$")
REQUIRED = ("id", "display", "nation", "status", "source_scheme", "identifier_name",
            "issuer_match", "identity", "subject_cn_format", "trust_anchor", "revocation",
            "example", "sources", "unverified")
# What the collab front calls each source. san_othername has no ALB-side reader yet.
TERRAFORM_IDENTITY = {"san_upn": "san_upn", "san_email": "san_email", "cn_regex": "cn", "serial": "serial"}
# The nginx variable each source is read from. The three SAN sources come from
# pki.js (ops/ansible/roles/icam-mtls-proxy/files/pki.js) over $ssl_client_raw_cert.
NGINX_SOURCE_VAR = {
    "san_upn": "$pki_san_upn",
    "san_email": "$pki_san_emails",
    "cn_regex": "$pki_subject_cn",
    "serial": "$ssl_client_serial",
}


def load(path=REGISTRY):
    data = yaml.safe_load(pathlib.Path(path).read_text(encoding="utf-8")) or {}
    return data


def pkis(reg):
    return list(reg.get("pkis") or [])


def live(reg):
    return [p for p in pkis(reg) if p.get("status") == "live"]


def by_id(reg, pki_id):
    for p in pkis(reg):
        if p.get("id") == pki_id:
            return p
    raise KeyError(f"no PKI {pki_id!r} in the registry")


def nginx_var(pki_id):
    return pki_id.replace("-", "_")


# --- validation -----------------------------------------------------------------
def _group_count(pattern):
    return re.compile(pattern).groups


def validate(reg, grammar_schemes=None):
    """Return a list of problems (empty = clean)."""
    problems = []
    if reg.get("version") != 1:
        problems.append("registry: version must be 1")
    seen = set()
    for i, p in enumerate(pkis(reg)):
        tag = f"pkis[{i}] ({p.get('id', '?')})"
        for f in REQUIRED:
            if f not in p:
                problems.append(f"{tag}: missing field {f}")
        pid = p.get("id", "")
        if not ID_RE.match(str(pid)):
            problems.append(f"{tag}: id must be a lower-case slug")
        if pid in seen:
            problems.append(f"{tag}: duplicate id")
        seen.add(pid)
        if not NATION_RE.match(str(p.get("nation", ""))):
            problems.append(f"{tag}: nation must be ISO 3166-1 alpha-3 or NATO")
        status = p.get("status")
        if status not in STATUSES:
            problems.append(f"{tag}: status must be one of {STATUSES}")
        if grammar_schemes is not None and p.get("source_scheme") not in grammar_schemes:
            problems.append(f"{tag}: source_scheme {p.get('source_scheme')!r} is not a grammar.yaml identity.source_scheme value")
        if not isinstance(p.get("sources"), list) or not p.get("sources"):
            problems.append(f"{tag}: sources must list at least one entry with url and date")
        else:
            for s in p["sources"]:
                if not isinstance(s, dict) or not {"what", "url", "date"} <= set(s):
                    problems.append(f"{tag}: every source needs what, url, date")
        unverified = p.get("unverified")
        if not isinstance(unverified, list):
            problems.append(f"{tag}: unverified must be a list (empty when everything is verified)")
            unverified = []
        if status == "live":
            if unverified:
                problems.append(f"{tag}: a live profile may not carry unverified fields: {unverified}")
            for f in ("issuer_match", "identity", "subject_cn_format", "example"):
                if p.get(f) is None:
                    problems.append(f"{tag}: a live profile needs {f}")
            ta = p.get("trust_anchor") or {}
            for f in ("publisher", "source_url", "verification", "refresh", "load"):
                if not ta.get(f) or ta.get(f) == "unverified":
                    problems.append(f"{tag}: trust_anchor.{f} must be stated for a live profile")
            rv = p.get("revocation") or {}
            if "crl" not in rv or "ocsp" not in rv:
                problems.append(f"{tag}: revocation needs crl and ocsp (a URL, a pattern or none)")
        # Regexes and the identity rule, wherever present.
        if p.get("issuer_match") is not None:
            try:
                re.compile(p["issuer_match"])
            except re.error as e:
                problems.append(f"{tag}: issuer_match does not compile: {e}")
        ident = p.get("identity")
        if ident is not None:
            src = ident.get("source")
            if src not in SOURCES:
                problems.append(f"{tag}: identity.source must be one of {SOURCES}")
            if src == "san_othername" and not ident.get("oid"):
                problems.append(f"{tag}: identity.oid is required for san_othername")
            rx = ident.get("identifier_regex")
            if src == "cn_regex" and not rx:
                problems.append(f"{tag}: identity.identifier_regex is required for cn_regex")
            if rx:
                try:
                    if _group_count(rx) != 1:
                        problems.append(f"{tag}: identity.identifier_regex must have exactly one capture group")
                except re.error as e:
                    problems.append(f"{tag}: identity.identifier_regex does not compile: {e}")
        ta = p.get("trust_anchor") or {}
        if ta.get("load") not in ("issuing", "all"):
            problems.append(f"{tag}: trust_anchor.load must be issuing or all")
        if ta.get("load") == "issuing" and status == "live" and not ta.get("issuing_ca_match"):
            problems.append(f"{tag}: trust_anchor.issuing_ca_match is required when load is issuing")
        scf = p.get("subject_cn_format")
        if scf is not None:
            if not scf.get("pattern"):
                problems.append(f"{tag}: subject_cn_format.pattern is required")
            if scf.get("regex"):
                try:
                    re.compile(scf["regex"])
                except re.error as e:
                    problems.append(f"{tag}: subject_cn_format.regex does not compile: {e}")
        # The example must satisfy the profile's own rules.
        ex = p.get("example")
        if ex is not None and ident is not None and p.get("issuer_match"):
            if not re.search(p["issuer_match"], ex.get("issuer_dn", "")):
                problems.append(f"{tag}: example.issuer_dn does not match issuer_match")
            got = identifier(p, leaf_from_example(ex))
            if got != ex.get("identifier"):
                problems.append(f"{tag}: the identity rule yields {got!r} from the example, not {ex.get('identifier')!r}")
            if scf and scf.get("regex"):
                cn = rfc2253_cn(ex.get("subject_dn", ""))
                m = re.search(scf["regex"], cn)
                if not m or m.group(1) != ex.get("identifier"):
                    problems.append(f"{tag}: subject_cn_format.regex does not yield the identifier from the example subject CN {cn!r}")
    return problems


# --- the identity rule, in Python --------------------------------------------------
def rfc2253_cn(dn):
    """The first CN value of an RFC 2253 DN, with its escapes removed."""
    m = re.match(r"CN=((?:[^,\\]|\\.)*)", dn)
    if not m:
        return ""
    return re.sub(r"\\(.)", r"\1", m.group(1))


def leaf_from_example(ex):
    """The leaf dict identifier() reads, built from a registry example."""
    return {
        "subject": ex.get("subject_dn", ""),
        "subject_cn": rfc2253_cn(ex.get("subject_dn", "")),
        "issuer": ex.get("issuer_dn", ""),
        "serial": ex.get("serial", "01"),
        "san_upn": [ex["san_upn"]] if ex.get("san_upn") else [],
        "san_email": [ex["san_email"]] if ex.get("san_email") else [],
        "san_othername": dict(ex.get("san_othername") or {}),
    }


def raw_value(pki, leaf):
    ident = pki["identity"]
    src = ident["source"]
    if src == "san_upn":
        return leaf["san_upn"][0] if leaf.get("san_upn") else None
    if src == "san_email":
        return leaf["san_email"][0] if leaf.get("san_email") else None
    if src == "san_othername":
        return (leaf.get("san_othername") or {}).get(ident["oid"])
    if src == "cn_regex":
        return leaf.get("subject_cn") or None
    if src == "serial":
        return leaf.get("serial")
    return None


def identifier(pki, leaf):
    """Apply the PKI's identity rule to a parsed leaf. None = no identifier (refuse)."""
    raw = raw_value(pki, leaf)
    if not raw:
        return None
    rx = pki["identity"].get("identifier_regex")
    if rx:
        m = re.search(rx, raw)
        return m.group(1) if m else None
    if pki["identity"]["source"] == "san_upn":
        return raw.split("@", 1)[0]
    return raw


# --- nginx --------------------------------------------------------------------------
def nginx_quote(s):
    """Quote a string for an nginx config: the config reader turns \\\\ into \\ and
    \\" into \", so a regex backslash pair is doubled and a quote is escaped."""
    return '"' + s.replace("\\\\", "\\\\\\\\").replace('"', '\\"') + '"'


def _source_var(pki):
    ident = pki["identity"]
    if ident["source"] == "san_othername":
        return f"$pki_othername_{nginx_var(pki['id'])}"
    return NGINX_SOURCE_VAR[ident["source"]]


def _identifier_regex(pki):
    ident = pki["identity"]
    rx = ident.get("identifier_regex")
    if rx:
        return rx
    if ident["source"] == "san_upn":
        return r"^([^@]+)@"
    return r"^(.+)$"


def nginx_maps(selected):
    """The map blocks for the selected profiles. Every variable is empty unless the
    client certificate matched exactly one profile AND yielded an identifier, so a
    location can refuse on `$pki_remote_user = ""`."""
    if not selected:
        raise ValueError("no PKI profile selected")
    out = []
    out.append("# Generated from ops/identity/sovereign-pkis.yaml (sovereign_pkis.nginx_maps). Do not edit.")
    out.append("# $pki_id            which profile issued the client certificate (issuer DN match)")
    out.append("# $pki_identifier    the identifier that profile's rule read from the certificate")
    out.append("# $pki_remote_user   \"<pki>:<identifier>\", or \"\" when either is missing")
    out.append("# $pki_san_email     the card's first SAN e-mail, or \"\"")
    out.append("map $ssl_client_i_dn $pki_id {")
    out.append('    default "";')
    for p in selected:
        out.append(f"    {nginx_quote('~' + p['issuer_match'])} {p['id']};")
    out.append("}")
    # The subject CN as a value of its own (RFC 2253 escapes kept), for cn_regex rules.
    # The first SAN e-mail on its own, for X-Remote-Email (Dex takes one address).
    out.append("map $pki_san_emails $pki_san_email {")
    out.append('    default "";')
    out.append('    ' + nginx_quote("~^([^;]+)") + " $1;")
    out.append("}")
    out.append("map $ssl_client_s_dn $pki_subject_cn {")
    out.append('    default "";')
    out.append('    ' + nginx_quote(r"~^CN=((?:[^,\\]|\\.)*)") + " $1;")
    out.append("}")
    for p in selected:
        v = nginx_var(p["id"])
        out.append(f"map {_source_var(p)} $pki_identifier_{v} {{")
        out.append('    default "";')
        out.append(f"    {nginx_quote('~' + _identifier_regex(p))} $1;")
        out.append("}")
    out.append("map $pki_id $pki_identifier {")
    out.append('    default "";')
    for p in selected:
        v = nginx_var(p["id"])
        out.append(f"    {p['id']} $pki_identifier_{v};")
    out.append("}")
    out.append('map "$pki_id:$pki_identifier" $pki_remote_user {')
    out.append('    default $pki_id:$pki_identifier;')
    out.append('    "~^:" "";')
    out.append('    "~:$" "";')
    out.append("}")
    return "\n".join(out) + "\n"


def nginx_js_set_lines(selected):
    """The js_set directives the maps need (http context)."""
    lines = ["js_set $pki_san_upn pki.sanUpn;", "js_set $pki_san_emails pki.sanEmails;"]
    for p in selected:
        if p["identity"]["source"] == "san_othername":
            lines.append(f"js_set $pki_othername_{nginx_var(p['id'])} profiles.othername_{nginx_var(p['id'])};")
    return lines


def nginx_profiles_js(selected):
    """pki-profiles.js: one wrapper per san_othername profile, or None when none."""
    others = [p for p in selected if p["identity"]["source"] == "san_othername"]
    if not others:
        return None
    out = ["// Generated from ops/identity/sovereign-pkis.yaml (sovereign_pkis.nginx_profiles_js). Do not edit.",
           "import pki from 'pki.js';", ""]
    for p in others:
        out.append(f"function othername_{nginx_var(p['id'])}(r) {{ return pki.otherName(r, '{p['identity']['oid']}'); }}")
    out.append("")
    out.append("export default { " + ", ".join(f"othername_{nginx_var(p['id'])}" for p in others) + " };")
    return "\n".join(out) + "\n"


# --- collab-services (Terraform) -----------------------------------------------------
def _hcl(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def terraform_value(selected):
    """The HCL list for var.sovereign_pkis in collab-services
    environments/aws/prod/main_cert_signin.tf."""
    for p in selected:
        if p["identity"]["source"] not in TERRAFORM_IDENTITY:
            raise ValueError(f"{p['id']}: identity source {p['identity']['source']} has no cert-front reader yet")
    out = ["  default = ["]
    for p in selected:
        rows = [("id", p["id"]), ("display", p["display"]), ("issuer_match", p["issuer_match"]),
                ("identity", TERRAFORM_IDENTITY[p["identity"]["source"]])]
        if p["identity"].get("identifier_regex"):
            rows.append(("identifier_regex", p["identity"]["identifier_regex"]))
        rows.append(("bundle_path", f"pki/{p['id']}/ca-bundle.pem"))
        width = max(len(k) for k, _ in rows)
        out.append("    {")
        out.append(f"      # {p['display']}: identifier = {p['identifier_name']}. Generated from USAREUR-AF/goatcore")
        out.append("      # ops/identity/sovereign-pkis.yaml by ops/identity/sovereign-pkis-render.py terraform; edit there.")
        for k, v in rows:
            out.append(f"      {k.ljust(width)} = {_hcl(v)}")
        out.append("    },")
    out.append("  ]")
    return "\n".join(out) + "\n"


# --- the store (OpenBao) -------------------------------------------------------------
def openbao_layout(selected):
    """{relative path: content or None}. None = the PKI's CA bundle, which the
    operator places (bootstrap.sh 3d reads <id>/ca-bundle.pem and nothing else)."""
    layout = {}
    for p in selected:
        ta = p["trust_anchor"]
        layout[f"{p['id']}/ca-bundle.pem"] = None
        layout[f"{p['id']}/SOURCE"] = (
            f"{p['display']}\n"
            f"ca-bundle.pem: {ta.get('loaded', 'the issuing CAs and their roots')}\n"
            f"from: {ta['source_url']}\n"
            f"version: {ta.get('version', 'n/a')}\n"
            f"verify: {ta['verification'].strip()}\n"
            f"refresh: {ta['refresh']}\n"
            f"binding: people.py bind-card <name> --pki {p['id']} --cn \"{p['subject_cn_format']['pattern']}\"\n"
        )
    return layout


# --- Kratos claim map --------------------------------------------------------------------
def _jsonnet_str(s):
    return "'" + s.replace("\\", "\\\\").replace("'", "\\'") + "'"


def claim_map(selected):
    """sovereign-pki.jsonnet: the Kratos OIDC data mapper for the mtls_proxy mode.
    preferred_username arrives as "<pki>:<identifier>" (the proxy's X-Remote-User)."""
    out = [
        "// sovereign-pki.jsonnet — Kratos OIDC data mapper for card sign-in through the",
        "// icam-mtls-proxy (ADR 1209). GENERATED from ops/identity/sovereign-pkis.yaml by",
        "// ops/identity/sovereign-pkis-render.py claim-map; CI refuses drift. Edit the registry.",
        "//",
        "// The proxy hands Dex X-Remote-User = \"<pki>:<identifier>\"; Dex carries it as",
        "// preferred_username. This mapper splits it, refuses an unknown profile, and records",
        "// the binding in metadata_public.external_subjects with pki + pki_subject. A profile",
        "// that names an alias claim (us-dod: edipi) gets it filled too, for readers that still",
        "// look for the old name. Nothing here reads or sets a nationality (ADR 1171).",
        "local c = std.extVar('claims').raw_claims;",
        "local client_id = 'goat-zagros';   // fixed per file: our RP client_id at the proxy's Dex",
        "",
        "// One row per live profile in the registry.",
        "local profiles = {",
    ]
    for p in selected:
        alias = "'edipi'" if p["id"] == "us-dod" else "null"
        email_domain = "mil" if p["id"] == "us-dod" else f"{p['id']}.card.invalid"
        out.append(f"  {_jsonnet_str(p['id'])}: {{ scheme: {_jsonnet_str(p['source_scheme'])}, "
                   f"identifier_name: {_jsonnet_str(p['identifier_name'])}, alias: {alias}, "
                   f"email_domain: {_jsonnet_str(email_domain)} }},")
    out += [
        "};",
        "",
        "local user = if std.objectHas(c, 'preferred_username') then c.preferred_username else c.sub;",
        "local sep = std.findSubstr(':', user);",
        "local pki = if std.length(sep) > 0 then std.substr(user, 0, sep[0]) else '';",
        "local pki_subject = if std.length(sep) > 0 then std.substr(user, sep[0] + 1, std.length(user) - sep[0] - 1) else '';",
        "// Fail closed: a username without a known profile prefix mints nothing.",
        "local profile = if std.objectHas(profiles, pki) && pki_subject != '' then profiles[pki]",
        "                else error 'card sign-in: unknown PKI profile or empty identifier in ' + user;",
        "",
        "{",
        "  // The card's own e-mail when it carries one; otherwise an address that cannot be",
        "  // delivered (the trait is required and must be unique). us-dod keeps <EDIPI>@mil.",
        "  local email = if std.objectHas(c, 'email') && std.length(std.findSubstr('@', c.email)) > 0",
        "                then c.email else pki_subject + '@' + profile.email_domain,",
        "  identity: {",
        "    // Rule 2: a card carries no goat authorization — role/affiliation are assigned locally.",
        "    traits: {",
        "      email: email,",
        "      name: if std.objectHas(c, 'name') then c.name else user,",
        "    },",
        "    metadata_public: {",
        "      // GHSA-9v3v: login groups live in admin-managed metadata_public (the",
        "      // migration marker); never in self-editable traits.",
        "      groups: [],",
        "      external_subjects: [{",
        "        issuer: c.iss,",
        "        client_id: client_id,",
        "        subject: user,              // \"<pki>:<identifier>\" — a mapped attribute, never the key",
        "        pki: pki,",
        "        pki_subject: pki_subject,",
        "        identifier_name: profile.identifier_name,",
        "        scheme: profile.scheme,",
        "        credential_type: 'x509-card',",
        "        last_assurance: {",
        "          ial: 'ial3',            // in-person issuance of a sovereign card",
        "          aal: 'aal3',            // hardware, phishing-resistant",
        "          fal: 'fal2',",
        "          acr: if std.objectHas(c, 'acr') then c.acr else '',",
        "          boundary_il: 5,",
        "        },",
        "      } + (if profile.alias != null then { [profile.alias]: pki_subject } else {})],",
        "    },",
        "  },",
        "}",
    ]
    return "\n".join(out) + "\n"


# --- a synthetic card in the PKI's shape (test PKI generator) -------------------------------
def _split_rdns(dn):
    """RFC 2253 DN -> list of (attr, value) in the order written (CN first)."""
    rdns = re.split(r"(?<!\\),", dn)
    out = []
    for rdn in rdns:
        k, _, v = rdn.partition("=")
        out.append((k.strip(), re.sub(r"\\(.)", r"\1", v)))
    return out


def openssl_request(pki, identifier=None, email=None):
    """(ca_subj, cnf_text) for a synthetic card: `-subj` for the test CA (the example
    issuer DN) and an openssl req config for the leaf, identifier substituted."""
    ex = pki["example"]
    ident = pki["identity"]
    new = identifier or ex["identifier"]
    old = ex["identifier"]
    subject = ex["subject_dn"].replace(old, new)
    san = []
    if ex.get("san_upn"):
        san.append("otherName:msUPN;UTF8:" + ex["san_upn"].replace(old, new))
    mail = email or (ex.get("san_email") or "").replace(old, new)
    if mail:
        san.append("email:" + mail)
    for oid, val in (ex.get("san_othername") or {}).items():
        san.append(f"otherName:{oid};UTF8:{val.replace(old, new)}")
    lines = ["[req]", "distinguished_name = dn", "req_extensions = ext", "prompt = no", "utf8 = yes", "string_mask = utf8only", "[dn]"]
    # openssl writes RDNs in file order; RFC 2253 lists them reversed, so reverse here.
    for i, (k, v) in enumerate(reversed(_split_rdns(subject))):
        key = k if re.match(r"^[A-Za-z]", k) else k  # dotted OIDs are accepted by openssl as-is
        lines.append(f"{i}.{key} = {v}")
    lines += ["[ext]", "keyUsage = critical, digitalSignature", "extendedKeyUsage = clientAuth, msSmartcardLogin"]
    if san:
        lines.append("subjectAltName = " + ", ".join(san))
    ca_subj = "/" + "/".join(f"{k}={v}" for k, v in reversed(_split_rdns(ex["issuer_dn"])))
    return ca_subj, "\n".join(lines) + "\n", {"identifier": new, "source": ident["source"]}
