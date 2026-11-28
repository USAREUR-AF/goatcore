"""A host enrolled for a site but not of its group (the goatnet's relay-health
box) enrols where the site's hosts do, without stating the URL a second time."""
import pathlib
import re
import unittest

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/mesh-enroll-host.yml"


class EnrolUrl(unittest.TestCase):
    def setUp(self):
        self.text = PLAY.read_text()
        self.play = [p for p in yaml.safe_load(self.text) if "_me_url" in (p.get("vars") or {})][0]

    def test_the_url_falls_back_to_the_minting_groups_first_host(self):
        url = self.play["vars"]["_me_url"]
        self.assertIn("mesh_enrollment_url | default(hostvars[_me_first_host]['mesh_enrollment_url']", url)
        self.assertIn("groups[mesh_enroll_mint_group | default(target)][0]", self.play["vars"]["_me_first_host"])

    def test_nothing_reads_the_groups_own_variable_past_that_line(self):
        code = "\n".join(l for l in self.text.split("\n") if not l.lstrip().startswith("#") and "_me_url:" not in l)
        self.assertEqual(re.findall(r"\bmesh_enrollment_url\s+is defined|management-url', mesh_enrollment_url", code), [])
        self.assertIn("'--management-url', _me_url,", code)

    def test_an_unset_url_is_still_refused(self):
        task = [t for t in self.play["tasks"] if "declares where its hosts enrol" in t.get("name", "")][0]
        self.assertIn("(_me_url_parts | length) == 2", task["ansible.builtin.assert"]["that"])


if __name__ == "__main__":
    unittest.main()
