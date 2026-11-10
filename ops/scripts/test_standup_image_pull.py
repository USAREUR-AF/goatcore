#!/usr/bin/env python3
"""Tests for the standup image pre-pull (docs/design/goatlite.md §11, measure 2).

1. standup-images.py lists the image pins of a member from the merged table: no
   template left, no duplicate, no component declared absent, non-images left out.
2. A lite sidecar's `absent` deviation removes that component's image (negative
   control against a copy of the table).
3. standup-image-pull.sh pulls every image with at most N at once, and an image
   that cannot be pulled fails the run naming it, after the others finished
   (negative control, a fake `docker` on PATH).
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
IMAGES = os.path.join(HERE, "standup-images.py")
PULL = os.path.join(HERE, "standup-image-pull.sh")

sys.path.insert(0, os.path.join(REPO, "ops", "consistency", "python"))
from goat_consistency import resolve  # noqa: E402


def _checkout_org():
    """The namespace standup-images.py resolves `{{ ghcr_org }}` to in this
    checkout, read through its own default_ghcr_org: an overlay's
    group_vars/all.yml may name another organisation than this repository's."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("standup_images", IMAGES)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.default_ghcr_org(REPO)


ORG = _checkout_org()
GOAT = "ghcr.io/%s/" % ORG


def run(*args, env=None):
    return subprocess.run(list(args), capture_output=True, text=True, env=env)


class ImageList(unittest.TestCase):
    def test_a_governed_member_lists_its_image_pins(self):
        t = resolve.load_tables()
        member_of = resolve.validate(t.goatnets, t.components)
        member = sorted(member_of)[0]
        r = run(sys.executable, IMAGES, "--member", member, "--ghcr-org", "someorg")
        self.assertEqual(r.returncode, 0, r.stderr)
        refs = r.stdout.split()
        self.assertTrue(refs)
        self.assertEqual(len(refs), len(set(refs)), "each image once")
        self.assertFalse([x for x in refs if "{{" in x], "every template resolved")
        expect = set()
        for spec in t.components.values():
            if (spec.get("artifact_type") or "image") != "image":
                continue
            v, _ = resolve.resolve(spec, member, member_of[member])
            if v is not None:
                expect.add(str(v).replace("{{ ghcr_org }}", "someorg"))
        self.assertEqual(set(refs), expect)

    def test_an_ungoverned_member_lists_nothing_and_says_so(self):
        r = run(sys.executable, IMAGES, "--member", "no_such_site_hosts")
        self.assertEqual((r.returncode, r.stdout), (0, ""))
        self.assertIn("nothing to pre-pull", r.stderr)

    def test_a_component_declared_absent_on_a_goatlite_is_not_pulled(self):
        d = tempfile.mkdtemp()
        try:
            shutil.copy(os.path.join(REPO, "ops/pins/fleet-pins.yaml"), os.path.join(d, "fleet-pins.yaml"))
            fleet = resolve.load_tables(os.path.join(d, "fleet-pins.yaml"), os.path.join(d, "missing.yaml"))
            present = [c for c, s in fleet.components.items() if (s.get("artifact_type") or "image") == "image" and s.get("fleet")]
            gone, kept = present[0], present[1]
            with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
                # hobby_hosts: a committed group_vars directory no pin table governs.
                # A component with no fleet value needs a deviation, as the render writes.
                off = [gone] + [c for c, s in fleet.components.items() if s.get("fleet") is None]
                f.write("goatnets:\n  hobby:\n    description: a test goatlite\n    members: [hobby_hosts]\ncomponents:\n")
                for c in off:
                    f.write("  %s:\n    deviations:\n      hobby:\n        absent: true\n        reason: off on one box\n" % c)
            r = run(sys.executable, IMAGES, "--member", "hobby_hosts", "--table", os.path.join(d, "fleet-pins.yaml"))
            self.assertEqual(r.returncode, 0, r.stderr)
            refs = r.stdout.split()
            fleet_ref = lambda c: str(fleet.components[c]["fleet"]).replace("{{ ghcr_org }}", ORG)
            self.assertNotIn(fleet_ref(gone), refs)
            self.assertIn(fleet_ref(kept), refs)
        finally:
            shutil.rmtree(d)


def renderer():
    import importlib.util
    spec = importlib.util.spec_from_file_location("goatlite_render", os.path.join(HERE, "goatlite-render.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class LitePreset(unittest.TestCase):
    """`--preset lite` (the prepared VM image's list, goatlite.md §11 measure 5) is what a
    rendered goatlite's pins resolve to; lite.image_registry moves goat's images only."""

    def table_with_rendered_goatlite(self, d, registry=""):
        """A copy of the fleet table with a lite sidecar holding the deviations
        goatlite-render.py writes, for a goatlite `hobby` (member hobby_hosts: a committed group_vars directory no pin table governs)."""
        import yaml
        shutil.copy(os.path.join(REPO, "ops/pins/fleet-pins.yaml"), os.path.join(d, "fleet-pins.yaml"))
        comps = {}
        for cname, dev in renderer().lite_deviations(registry).items():
            entry = {"absent": True, "reason": "test"} if dev[0] == "absent" else {"value": dev[1], "reason": "test"}
            comps[cname] = {"deviations": {"hobby": entry}}
        with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
            yaml.safe_dump({"goatnets": {"hobby": {"description": "a test goatlite", "members": ["hobby_hosts"],
                                                      "ca_id_var": "goat_ca_id"}},
                            "components": comps}, f)
        return os.path.join(d, "fleet-pins.yaml")

    def test_preset_lite_lists_what_a_rendered_goatlite_runs(self):
        d = tempfile.mkdtemp()
        try:
            table = self.table_with_rendered_goatlite(d)
            member = run(sys.executable, IMAGES, "--member", "hobby_hosts", "--table", table)
            preset = run(sys.executable, IMAGES, "--preset", "lite")
            self.assertEqual(member.returncode, 0, member.stderr)
            self.assertEqual(preset.returncode, 0, preset.stderr)
            self.assertTrue(preset.stdout.split())
            self.assertEqual(sorted(preset.stdout.split()), sorted(member.stdout.split()))
        finally:
            shutil.rmtree(d)

    def test_image_registry_moves_goat_images_and_nothing_else(self):
        reg = "registry.lan:5000/goat"
        d = tempfile.mkdtemp()
        try:
            table = self.table_with_rendered_goatlite(d, reg)
            plain = run(sys.executable, IMAGES, "--preset", "lite").stdout.split()
            moved = run(sys.executable, IMAGES, "--preset", "lite", "--image-registry", reg).stdout.split()
            member = run(sys.executable, IMAGES, "--member", "hobby_hosts", "--table", table).stdout.split()
            self.assertEqual(sorted(moved), sorted(member), "the rendered pins are the listed ones")
            self.assertEqual(len(plain), len(moved))
            goat = [r for r in plain if r.startswith(GOAT)]
            other = [r for r in plain if not r.startswith(GOAT)]
            self.assertTrue(goat and other, "the lite set has goat images and others (control)")
            self.assertEqual(sorted(r.replace(GOAT, reg + "/", 1) for r in goat),
                             sorted(r for r in moved if r.startswith(reg + "/")), "same name and digest")
            self.assertEqual(sorted(other), sorted(r for r in moved if not r.startswith(reg + "/")),
                             "another project's image keeps its registry")
        finally:
            shutil.rmtree(d)

    def test_registry_vars_only_for_a_registry_off_the_box(self):
        r = renderer()
        self.assertEqual(r.registry_vars({"image_registry": ""}), "")
        self.assertEqual(r.registry_vars({"image_registry": "127.0.0.1:5000/usareur-af"}), "")
        self.assertEqual(r.registry_vars({"image_registry": "localhost:5000/usareur-af"}), "")
        self.assertIn('docker_engine_insecure_registries: ["registry.lan:5000"]',
                      r.registry_vars({"image_registry": "registry.lan:5000/usareur-af"}))
        self.assertIsNotNone(r.IMAGE_REGISTRY.match("127.0.0.1:5000/usareur-af"))
        self.assertIsNone(r.IMAGE_REGISTRY.match("127.0.0.1:5000"), "a namespace is required")


class Pull(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.d, "bin"))
        fake = os.path.join(self.d, "bin", "docker")
        # A pull takes 1 s; it records how many pulls run at once; an image
        # whose name contains "gone" fails as a registry 404 would.
        with open(fake, "w") as f:
            f.write("""#!/usr/bin/env bash
img="${@: -1}"; lock="%s/running"
mkdir -p "$lock"; touch "$lock/$$"; ls "$lock" | wc -l >> "%s/peaks"
sleep 1; rm -f "$lock/$$"
case "$img" in *gone*) echo "Error response from daemon: manifest unknown" >&2; exit 1 ;; esac
echo "$img"
""" % (self.d, self.d))
        os.chmod(fake, 0o700)
        self.env = dict(os.environ, PATH=os.path.join(self.d, "bin") + os.pathsep + os.environ["PATH"])

    def tearDown(self):
        shutil.rmtree(self.d)

    def peak(self):
        with open(os.path.join(self.d, "peaks")) as f:
            return max(int(x) for x in f.read().split())

    def test_every_image_is_pulled_at_most_n_at_once(self):
        t0 = time.time()
        r = run("bash", PULL, "--concurrency", "2", "a:1", "b:1", "c:1", "d:1", env=self.env)
        dt = time.time() - t0
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("4 of 4 image(s) present", r.stdout)
        self.assertLessEqual(self.peak(), 2, "never more than --concurrency at once")
        self.assertGreaterEqual(dt, 2.0, "four 1 s pulls two at a time take at least 2 s")
        self.assertLess(dt, 3.9, "and run in parallel, not one after another")

    def test_an_image_that_cannot_be_pulled_fails_the_run_by_name(self):
        r = run("bash", PULL, "--concurrency", "4", "a:1", "ghcr.io/x/gone:1", "c:1", env=self.env)
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("could not pull ghcr.io/x/gone:1: Error response from daemon: manifest unknown", r.stdout)
        self.assertIn("2 of 3 image(s) present", r.stdout)
        self.assertNotIn("could not pull a:1", r.stdout)

    def test_a_digest_pinned_image_already_on_the_host_is_not_pulled(self):
        # `docker image inspect` succeeds for refs containing "had"; the fake records
        # every pull in "pulls". A digest ref the host holds is skipped; a tag ref and a
        # digest ref the host lacks are pulled (negative control).
        d = "@sha256:" + "a" * 64
        with open(os.path.join(self.d, "bin", "docker"), "w") as f:
            f.write("""#!/usr/bin/env bash
if [ "$1 $2" = "image inspect" ]; then case "$3" in *had*) exit 0 ;; *) exit 1 ;; esac; fi
echo "${@: -1}" >> "%s/pulls"
""" % self.d)
        r = run("bash", PULL, "ghcr.io/x/had" + d, "ghcr.io/x/had:1", "ghcr.io/x/new" + d, env=self.env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("3 of 3 image(s) present", r.stdout)
        self.assertIn("(1 already on the host by digest, not pulled)", r.stdout)
        with open(os.path.join(self.d, "pulls")) as f:
            pulled = f.read().split()
        self.assertEqual(sorted(pulled), sorted(["ghcr.io/x/had:1", "ghcr.io/x/new" + d]))

    def test_usage_is_refused(self):
        self.assertEqual(run("bash", PULL, env=self.env).returncode, 2)
        self.assertEqual(run("bash", PULL, "--concurrency", "0", "a:1", env=self.env).returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
