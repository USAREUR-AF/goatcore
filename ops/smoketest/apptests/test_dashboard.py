"""Tests for the Netbird and smoketest dashboards."""

import pytest
import requests


def test_dashboard_reachable():
    """GET https://netbird.legacy.example returns 200."""
    resp = requests.get("https://netbird.legacy.example", timeout=10)
    assert resp.status_code == 200


def test_dashboard_html():
    """Dashboard response contains expected HTML content."""
    resp = requests.get("https://netbird.legacy.example", timeout=10)
    assert resp.status_code == 200
    assert "text/html" in resp.headers.get("Content-Type", "")
    assert "<html" in resp.text.lower()


def test_smoketest_dashboard():
    """GET http://smoketest.netbird.legacy.example returns 200."""
    try:
        resp = requests.get(
            "http://smoketest.netbird.legacy.example", timeout=10
        )
    except requests.ConnectionError:
        return pytest.skip("smoketest dashboard not deployed or not reachable")
    assert resp.status_code == 200
    return None
