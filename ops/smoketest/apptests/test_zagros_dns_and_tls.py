"""ZAGROS public DNS and TLS e2e checks (GH #151 fix verification).

These tests are intended to run from a host with normal internet
connectivity (ideally a mesh-connected controller). They validate that
the public intake and zenoh-door hostnames resolve directly to the
relay EIPs and that Traefik presents the correct certificates for
those SNI names, without leaking the netbird apex or mesh IP into the
SAN set.
"""

import ipaddress
import socket
import ssl

import pytest


DOMAIN = "zagros.example"
INTAKE_FQDN = f"intake.{DOMAIN}"
ZENOH_DOOR_FQDN = f"zenoh-door.{DOMAIN}"


def _resolve_ipv4_set(name: str) -> set[str]:
    """Resolve name to a set of IPv4 addresses or skip when unreachable."""

    try:
        results = socket.getaddrinfo(name, 443, socket.AF_INET)
    except OSError as e:  # DNS or network unreachable in this environment
        return pytest.skip(f"DNS lookup for {name} failed in this environment: {e}")
    ips = {r[4][0] for r in results}
    if not ips:
        pytest.skip(f"{name} resolved to no IPv4 addresses in this environment")
    return ips


def _fetch_cert(host: str, port: int = 443) -> dict:
    """Fetch the peer certificate for host:port or skip on reachability errors."""

    ctx = ssl.create_default_context()
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_REQUIRED
    try:
        with socket.create_connection((host, port), timeout=8) as sock:
            with ctx.wrap_socket(sock, server_hostname=host) as ssock:
                cert = ssock.getpeercert()
    except (OSError, ssl.SSLError) as e:
        return pytest.skip(f"TLS handshake to {host}:{port} not reachable in this environment: {e}")
    if not cert:
        raise AssertionError(f"Server at {host}:{port} presented no certificate")
    return cert


def _san_dns_names(cert: dict) -> list[str]:
    return [value for (kind, value) in cert.get("subjectAltName", []) if kind == "DNS"]


def test_intake_and_zenoh_door_resolve_to_five_public_ips():
    """intake/zenoh-door round-robin directly over the 5 relay EIPs.

    This asserts two properties:
      * each hostname resolves to exactly 5 IPv4 addresses, and
      * every IP is globally routable (no mesh-only/CGNAT addresses),
        which would include the Thor mesh IP that GH #151 describes.
    """

    intake_ips = _resolve_ipv4_set(INTAKE_FQDN)
    zenoh_ips = _resolve_ipv4_set(ZENOH_DOOR_FQDN)

    assert len(intake_ips) == 5, f"{INTAKE_FQDN} expected 5 IPs, got {len(intake_ips)}: {intake_ips}"
    assert len(zenoh_ips) == 5, f"{ZENOH_DOOR_FQDN} expected 5 IPs, got {len(zenoh_ips)}: {zenoh_ips}"

    # The two names should share the same relay EIP pool.
    assert intake_ips == zenoh_ips, (
        f"intake and zenoh-door should share the same 5 relay EIPs; "
        f"intake={intake_ips}, zenoh-door={zenoh_ips}"
    )

    for ip in intake_ips | zenoh_ips:
        addr = ipaddress.ip_address(ip)
        assert addr.version == 4, f"expected IPv4 address, got {ip}"
        # Global excludes RFC1918, CGNAT (100.64/10), and other non-public ranges.
        assert addr.is_global, f"expected public relay EIP, got non-global address {ip}"


def test_intake_tls_terminates_with_intake_certificate():
    """HTTPS to intake.zagros.example terminates with an intake cert.

    Verifies that traffic to the intake hostname is routed directly to
    Traefik over the relay EIPs and that the served certificate's SAN
    set includes the intake hostname, with no apex leakage.
    """

    cert = _fetch_cert(INTAKE_FQDN, 443)
    sans = _san_dns_names(cert)

    assert INTAKE_FQDN in sans, (
        f"expected {INTAKE_FQDN} in certificate SANs, got {sans}"
    )
    assert not any("netbird." in san for san in sans), (
        f"intake certificate SANs should not leak the netbird apex; got {sans}"
    )


def test_zenoh_door_tls_terminates_with_zenoh_door_certificate():
    """TLS to zenoh-door.zagros.example presents a zenoh-door cert.

    The outer mTLS lane should still terminate at Traefik with a
    certificate whose SANs are keyed on zenoh-door, not the apex.
    This proves that switching from a CNAME to direct A records did
    not change the observable TLS identity.
    """

    cert = _fetch_cert(ZENOH_DOOR_FQDN, 443)
    sans = _san_dns_names(cert)

    assert ZENOH_DOOR_FQDN in sans, (
        f"expected {ZENOH_DOOR_FQDN} in certificate SANs, got {sans}"
    )
    assert not any("netbird." in san for san in sans), (
        f"zenoh-door certificate SANs should not leak the netbird apex; got {sans}"
    )
