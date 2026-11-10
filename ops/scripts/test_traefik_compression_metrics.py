#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Tests for traefik-compression-metrics.py.

The exporter's whole job is an honest compression ratio, and the way it goes
wrong is by folding in a response whose origin size it cannot measure. Traefik
logs OriginContentSize=0 for a chunked or streamed response (no Content-Length);
dividing by that manufactures phantom expansion. Measured on ZAGROS 2026-09-21:
the portal's mesh ratio read -15.9% and the gateway class read -4865%, entirely
from chunked /api/orgs responses carrying wire bytes over a zero origin.

These tests pin two behaviours:
  1. parse_records skips a record with OriginContentSize <= 0, the same way it
     skips a missing field, and counts it as skipped.
  2. accumulate scopes the eligible ratio and the missed count to what
     goat-compress can act on (over the 1024-byte floor, compressible type, not
     an excluded type).

Run: python3 -m unittest discover -s ops/scripts -p 'test_*.py'
"""
import importlib.util
import json
import pathlib
import unittest

HERE = pathlib.Path(__file__).parent
_spec = importlib.util.spec_from_file_location(
    "tcm", HERE / "traefik-compression-metrics.py"
)
tcm = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(tcm)


def rec(o, d, ct="application/json", svc="portal@file", host="100.127.0.5"):
    return {
        "OriginContentSize": o,
        "DownstreamContentSize": d,
        "downstream_Content-Type": ct,
        "ServiceName": svc,
        "ClientHost": host,
    }


def _text(*objs, extra_lines=()):
    lines = [json.dumps(o) for o in objs]
    lines.extend(extra_lines)
    return "\n".join(lines)


class ParseRecords(unittest.TestCase):
    def test_skips_chunked_origin_zero(self):
        records, skipped = tcm.parse_records(_text(rec(10000, 2000), rec(0, 118778)))
        self.assertEqual(len(records), 1)
        self.assertEqual(skipped, 1)
        self.assertNotIn(0, [r["OriginContentSize"] for r in records])

    def test_skips_negative_origin(self):
        records, skipped = tcm.parse_records(_text(rec(-1, 5000)))
        self.assertEqual(records, [])
        self.assertEqual(skipped, 1)

    def test_skips_missing_field_and_bad_json_but_not_blank(self):
        text = _text(
            rec(2000, 500),
            extra_lines=('{"OriginContentSize": 3000}', "{bad json", "not-json-at-all"),
        )
        records, skipped = tcm.parse_records(text)
        self.assertEqual(len(records), 1)   # only the well-formed pair
        self.assertEqual(skipped, 2)        # missing-field + bad-json; the non-{ line is not a skip


class Eligible(unittest.TestCase):
    def test_size_floor(self):
        self.assertTrue(tcm.eligible("application/json", 1024))
        self.assertFalse(tcm.eligible("application/json", 1023))

    def test_content_types(self):
        self.assertTrue(tcm.eligible("text/html", 5000))
        self.assertFalse(tcm.eligible("text/event-stream", 5000))       # SSE, excluded
        self.assertFalse(tcm.eligible("application/grpc", 5000))        # grpc, excluded
        self.assertFalse(tcm.eligible("application/octet-stream", 5000))  # not compressible
        self.assertFalse(tcm.eligible("image/png", 5000))


class Accumulate(unittest.TestCase):
    def test_scopes_and_sums(self):
        records = [
            rec(10000, 2000),                                # eligible, compressed
            rec(14, 14),                                     # sub-floor, not eligible
            rec(5000, 5000),                                 # eligible, uncompressed -> missed
            rec(2000, 2050, ct="application/octet-stream"),  # incompressible, real, not eligible
        ]
        origin, wire, eo, ew, resp, missed = tcm.accumulate(records)
        k = ("portal@file", "mesh")
        self.assertEqual(origin[k], 17014)
        self.assertEqual(wire[k], 9064)
        self.assertEqual(eo[k], 15000)      # 10000 + 5000
        self.assertEqual(ew[k], 7000)       # 2000 + 5000
        self.assertEqual(missed[k], 5000)   # only the eligible-but-uncompressed one
        self.assertEqual(resp[("portal@file", "mesh", "true")], 1)
        self.assertEqual(resp[("portal@file", "mesh", "false")], 3)

    def test_client_class_split(self):
        _, _, _, _, resp, _ = tcm.accumulate([rec(2000, 500, host="172.18.0.1")])
        self.assertEqual(resp[("portal@file", "gateway", "true")], 1)

    def test_client_class_covers_the_whole_mesh_range(self):
        for host in ("100.127.0.5", "100.127.0.251", "100.127.1.5", "100.127.0.5"):
            self.assertEqual(tcm.client_class(host), "mesh", host)
        for host in ("203.0.113.219", "198.51.100.204", "100.x.0.1"):
            self.assertEqual(tcm.client_class(host), "other", host)


class RegressionChunkedNoLongerPoisonsRatio(unittest.TestCase):
    """A chunked response (o=0, d large) used to drag the ratio negative."""

    def _ratio(self, records):
        o, w, *_ = tcm.accumulate(records)
        k = ("portal@file", "mesh")
        return (o[k] - w[k]) / o[k] if o[k] else float("nan")

    def test_without_guard_negative_with_guard_positive(self):
        healthy = [rec(10000, 2000), rec(5000, 1500)]  # genuine compression
        chunked = rec(0, 118778)                        # the poison

        # Had the chunked record reached accumulate, the ratio goes negative:
        # wire bytes accrue over a zero-origin record.
        self.assertLess(self._ratio(healthy + [chunked]), 0)

        # parse_records drops it before accumulate ever sees it, so the real
        # ratio survives.
        kept, _ = tcm.parse_records(_text(*(healthy + [chunked])))
        self.assertGreater(self._ratio(kept), 0.5)


if __name__ == "__main__":
    unittest.main()
