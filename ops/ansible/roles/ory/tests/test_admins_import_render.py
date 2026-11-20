"""GHSA-9v3v: the `kratos identity import` admin templates write login groups
to admin-managed metadata_public, never to self-editable traits. Every reader
ignores traits.groups, so an imported admin with traits groups holds nothing.

Run: python3 -m unittest ops/ansible/roles/ory/tests/test_admins_import_render.py
"""
import json
import pathlib
import unittest

import jinja2
import yaml

ROOT = pathlib.Path(__file__).resolve().parents[4]
TEMPLATES = [
    ROOT / "ansible/roles/ory/templates/ory/admins.yaml.j2",
    ROOT / "ansible/templates/ory/admins.yaml.j2",
]
ADMINS = [
    {"email": "a@example.test", "name": "A", "password_hash": "$2a$12$x", "groups": ["PLATFORM-ADMIN", "IAM-ADMIN"]},
    {"email": "b@example.test", "name": "B", "password_hash": "$2a$12$y", "groups": []},
]


def render(path, admins):
    env = jinja2.Environment(undefined=jinja2.StrictUndefined)
    env.filters["to_json"] = json.dumps
    return yaml.safe_load(env.from_string(path.read_text()).render(site_admins=admins, site_name="stub"))


class AdminsImportRender(unittest.TestCase):
    def test_groups_land_in_metadata_public_not_traits(self):
        for path in TEMPLATES:
            with self.subTest(template=str(path.relative_to(ROOT))):
                rows = render(path, ADMINS)
                self.assertEqual(len(rows), 2)
                for row in rows:
                    self.assertNotIn("groups", row["traits"])
                self.assertEqual(rows[0]["metadata_public"]["groups"], ["PLATFORM-ADMIN", "IAM-ADMIN"])
                self.assertEqual(rows[1]["metadata_public"]["groups"], [])

    def test_role_template_marks_an_admin_without_groups_migrated(self):
        rows = render(TEMPLATES[0], [{"email": "c@example.test", "name": "C", "password_hash": "$2a$12$z"}])
        self.assertEqual(rows[0]["metadata_public"], {"groups": []})


if __name__ == "__main__":
    unittest.main()
