"""Tests for the Netbird management API."""

import requests


def test_accounts_requires_auth(api_base_url):
    """GET /api/accounts without a token must return 401."""
    resp = requests.get(f"{api_base_url}/api/accounts", timeout=10)
    assert resp.status_code == 401


def test_accounts_with_token(api_base_url, http_session):
    """GET /api/accounts with a valid token returns 200 and valid JSON."""
    resp = http_session.get(f"{api_base_url}/api/accounts")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)
    assert len(data) >= 1


def test_peers_list(api_base_url, http_session):
    """GET /api/peers returns a list with at least one peer."""
    resp = http_session.get(f"{api_base_url}/api/peers")
    assert resp.status_code == 200
    peers = resp.json()
    assert isinstance(peers, list)
    assert len(peers) >= 1, "Expected at least 1 enrolled peer"
    for peer in peers:
        assert "name" in peer
        assert "ip" in peer
        assert "connected" in peer


def test_peer_fields(api_base_url, http_session):
    """Verify peer objects contain expected fields."""
    resp = http_session.get(f"{api_base_url}/api/peers")
    assert resp.status_code == 200
    peers = resp.json()
    assert len(peers) >= 1

    expected_fields = {"id", "name", "ip", "connected", "os", "version"}
    peer = peers[0]
    missing = expected_fields - set(peer.keys())
    assert not missing, f"Peer object missing fields: {missing}"
