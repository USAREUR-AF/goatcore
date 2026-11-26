# SPDX-License-Identifier: Apache-2.0
"""The monitor page renders every status and feedback string as text, never as markup.

Runs the page's own script under Node with a stub DOM and a status document whose every
string (including the receiver's feedback, which comes from outside the node) is markup,
then checks the HTML the page builds. Skipped when Node is not installed.
"""
import copy
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import bn_monitor  # noqa: E402
import simple_bn_sender  # noqa: E402

MARK = '<img src=x onerror="alert(1)">'

HARNESS = r"""
const els = {sub: {textContent: ""}, nodes: {innerHTML: ""}};
globalThis.document = {getElementById: id => els[id]};
globalThis.setInterval = () => 0;
const docs = JSON.parse(require("fs").readFileSync(process.argv[2], "utf8"));
globalThis.fetch = async () => ({json: async () => docs});
const code = require("fs").readFileSync(process.argv[3], "utf8");
(0, eval)(code.replace(/\btick\(\);setInterval\(tick,5000\);\s*$/, ""));
tick().then(() => { process.stdout.write(JSON.stringify({html: els.nodes.innerHTML, sub: els.sub.textContent})); });
"""


def _hostile(value):
    """Every string in the document replaced by markup; keys left alone."""
    if isinstance(value, str):
        return MARK
    if isinstance(value, list):
        return [_hostile(v) for v in value]
    if isinstance(value, dict):
        return {k: _hostile(v) for k, v in value.items()}
    return value


def _feedback(hello, status, request):
    """Receiver feedback in the shape the sender writes it, taken through its own parser."""
    fb = simple_bn_sender.Feedback(["192.0.2.9"])
    for msg in (hello, status, request):
        fb.take(json.dumps(msg).encode(), ("192.0.2.9", 4000), "port:40099")
    return dict(fb.status(), port=40099)


def _status_docs():
    doc = copy.deepcopy(bn_monitor._STATUS_EXAMPLE[0])
    feed = doc["feeds"][0]
    feed["last_error"] = "x"
    feed["sac"] = feed["sic"] = feed["src_port"] = feed["update_period_s"] = "x"
    feed["live_targets_by_kind"] = {MARK: 1}
    feed["identity"] = {"data_class": "x", "declared": {"sac": "x", "sic": "x"},
                        "observed": [{"sac": "x", "sic": "x", "categories": [48, "x"], "count": 3, "last": None}],
                        "check": "x", "problems": ["x"]}
    feed["sim_status"].update(basis="operator", set_by="x", note="x", agrees=False)
    doc = _hostile(doc)
    doc["uptime_s"] = 100
    # What a receiver can actually send: every string field of hello, status and request.
    doc["feedback"] = _feedback({"type": "hello", "system": MARK, "contact": MARK, "version": MARK},
                                {"type": "status", "receiving": True, "sics": [1], "note": MARK},
                                {"type": "request", "sac": 7, "transport": MARK, "wg_public_key": MARK,
                                 "wg_endpoint": MARK, "note": MARK})
    doc["feedback"]["hello"]["msg"][MARK] = MARK
    return [doc]


@unittest.skipUnless(shutil.which("node"), "node not installed")
class MonitorPage(unittest.TestCase):
    def render(self, docs):
        # Extract the page's single inline script by index, not a tag-matching
        # regex: this test parses our own known page, and an HTML-tag regex here
        # only invites fragile edge cases.
        page = bn_monitor.PAGE
        low = page.lower()
        open_end = page.index(">", low.index("<script")) + 1
        script = page[open_end:low.index("</script", open_end)]
        with tempfile.TemporaryDirectory() as tmp:
            paths = {}
            for name, text in (("docs.json", json.dumps(docs)), ("page.js", script), ("harness.js", HARNESS)):
                paths[name] = os.path.join(tmp, name)
                with open(paths[name], "w") as f:
                    f.write(text)
            out = subprocess.run(["node", paths["harness.js"], paths["docs.json"], paths["page.js"]],
                                 capture_output=True, text=True, timeout=30)
        self.assertEqual(out.returncode, 0, out.stderr)
        return json.loads(out.stdout)

    def test_example_renders(self):
        doc = dict(bn_monitor._STATUS_EXAMPLE[0],
                   feedback=_feedback({"type": "hello", "system": "partner decoder"}, {"type": "status"},
                                      {"type": "request", "note": "please"}))
        got = self.render([doc])
        self.assertIn("TPQ-53 (notional)", got["html"])
        self.assertIn("partner decoder", got["html"])

    def test_every_string_is_text(self):
        got = self.render(_status_docs())
        html = got["html"]
        self.assertNotIn("<img", html.lower())
        self.assertNotIn('onerror="', html)
        self.assertIn("&lt;img", html)
        self.assertGreaterEqual(html.count("&lt;img"), 20)


if __name__ == "__main__":
    unittest.main()
