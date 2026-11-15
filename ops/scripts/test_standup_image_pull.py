#!/usr/bin/env python3
"""Tests for the standup image pre-pull (docs/design/goatlite.md §11, measure 2).

1. standup-images.py lists the image pins of a member from the merged table: no
   template left, no duplicate, no component declared absent, non-images left out.
2. A lite sidecar's `absent` deviation removes that component's image (negative
   control against a copy of the table).
3. standup-image-pull.sh pulls every image with at most N at once, and an image
   that cannot be pulled fails the run naming it, after the others finished
   (negative control, a fake `docker` on PATH).

The listing tests run against a checkout of their own in a temporary directory
(fixture_checkout): copies of the scripts under test and the pin resolver, the
fleet table's components, and a synthetic goatlite goatnet `zagros` whose member
`zagros_hosts` has a group_vars folder there. This checkout declares no goatnet
and keeps no member group_vars, so nothing here depends on what it holds.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REAL = os.path.normpath(os.path.join(HERE, "..", ".."))
PULL = os.path.join(HERE, "standup-image-pull.sh")

GOATNET, MEMBER = "zagros", "zagros_hosts"


def fixture_checkout():
    """A checkout of its own: the scripts and the resolver find their tree from their
    own path, so copies of them here read only what is here."""
    d = tempfile.mkdtemp(prefix="standup-image-pull-")
    for rel in ("ops/scripts/standup-images.py", "ops/scripts/goatlite-render.py",
                "ops/ansible/group_vars/all.yml"):
        os.makedirs(os.path.dirname(os.path.join(d, rel)), exist_ok=True)
        shutil.copy(os.path.join(REAL, rel), os.path.join(d, rel))
    shutil.copytree(os.path.join(REAL, "ops/consistency/python"), os.path.join(d, "ops/consistency/python"),
                    ignore=shutil.ignore_patterns("__pycache__"))
    # The fleet table's components, and no goatnet: the goatnet is the fixture's.
    import yaml
    with open(os.path.join(REAL, "ops/pins/fleet-pins.yaml"), encoding="utf-8") as f:
        components = (yaml.safe_load(f) or {}).get("components") or {}
    os.makedirs(os.path.join(d, "ops/pins"))
    with open(os.path.join(d, "ops/pins/fleet-pins.yaml"), "w", encoding="utf-8") as f:
        yaml.safe_dump({"goatnets": {}, "components": components}, f)
    os.makedirs(os.path.join(d, "ops/ansible/group_vars", MEMBER))
    with open(os.path.join(d, "ops/ansible/group_vars", MEMBER, "vars.yml"), "w") as f:
        f.write("# the synthetic goatnet zagros's member, for the image list tests\n")
    return d


FIX = fixture_checkout()
IMAGES = os.path.join(FIX, "ops/scripts/standup-images.py")
sys.path.insert(0, os.path.join(FIX, "ops", "consistency", "python"))
from goat_consistency import resolve  # noqa: E402


def tearDownModule():
    shutil.rmtree(FIX, ignore_errors=True)


def _checkout_org():
    """The namespace standup-images.py resolves `{{ ghcr_org }}` to in the fixture
    checkout, read through its own default_ghcr_org from the group_vars/all.yml
    copied there."""
    import importlib.util
    spec = importlib.util.spec_from_file_location("standup_images", IMAGES)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.default_ghcr_org(FIX)


ORG = _checkout_org()
GOAT = "ghcr.io/%s/" % ORG


def run(*args, env=None):
    return subprocess.run(list(args), capture_output=True, text=True, env=env)


def renderer():
    import importlib.util
    spec = importlib.util.spec_from_file_location("goatlite_render", os.path.join(FIX, "ops/scripts/goatlite-render.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def zagros_table(d, registry="", absent=()):
    """The fixture's fleet table, copied into `d`, with a lite sidecar beside it
    declaring the goatlite zagros (member zagros_hosts) and the deviations
    goatlite-render.py writes for it; each component in `absent` declared absent
    too. Returns the fleet table's path, for --table."""
    import yaml
    shutil.copy(os.path.join(FIX, "ops/pins/fleet-pins.yaml"), os.path.join(d, "fleet-pins.yaml"))
    comps = {}
    for cname, dev in renderer().lite_deviations(registry).items():
        entry = {"absent": True, "reason": "test"} if dev[0] == "absent" else {"value": dev[1], "reason": "test"}
        comps[cname] = {"deviations": {GOATNET: entry}}
    for cname in absent:
        comps[cname] = {"deviations": {GOATNET: {"absent": True, "reason": "off on one box"}}}
    with open(os.path.join(d, "lite-pins.yaml"), "w") as f:
        yaml.safe_dump({"goatnets": {GOATNET: {"description": "a test goatlite", "members": [MEMBER],
                                                "ca_id_var": "goat_ca_id"}},
                        "components": comps}, f)
    return os.path.join(d, "fleet-pins.yaml")


class ImageList(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.d, True)
        self.table = zagros_table(self.d)

    def test_a_governed_member_lists_its_image_pins(self):
        t = resolve.load_tables(self.table)
        member_of = resolve.validate(t.goatnets, t.components)
        self.assertEqual(member_of, {MEMBER: GOATNET})
        r = run(sys.executable, IMAGES, "--member", MEMBER, "--ghcr-org", "someorg", "--table", self.table)
        self.assertEqual(r.returncode, 0, r.stderr)
        refs = r.stdout.split()
        self.assertTrue(refs)
        self.assertEqual(len(refs), len(set(refs)), "each image once")
        self.assertFalse([x for x in refs if "{{" in x], "every template resolved")
        expect = set()
        for spec in t.components.values():
            if (spec.get("artifact_type") or "image") != "image":
                continue
            v, _ = resolve.resolve(spec, MEMBER, GOATNET)
            if v is not None:
                expect.add(str(v).replace("{{ ghcr_org }}", "someorg"))
        self.assertEqual(set(refs), expect)

    def test_an_ungoverned_member_lists_nothing_and_says_so(self):
        r = run(sys.executable, IMAGES, "--member", "no_such_site_hosts", "--table", self.table)
        self.assertEqual((r.returncode, r.stdout), (0, ""), r.stderr)
        self.assertIn("nothing to pre-pull", r.stderr)

    def test_a_component_declared_absent_on_a_goatlite_is_not_pulled(self):
        t = resolve.load_tables(self.table)
        listed = run(sys.executable, IMAGES, "--member", MEMBER, "--table", self.table).stdout.split()
        present = [c for c, s in t.components.items()
                   if (s.get("artifact_type") or "image") == "image" and s.get("fleet")
                   and str(s["fleet"]).replace("{{ ghcr_org }}", ORG) in listed]
        self.assertGreaterEqual(len(present), 2, "two components the goatlite runs at their fleet value")
        gone, kept = present[0], present[1]
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d, True)
        table = zagros_table(d, absent=[gone])
        r = run(sys.executable, IMAGES, "--member", MEMBER, "--table", table)
        self.assertEqual(r.returncode, 0, r.stderr)
        refs = r.stdout.split()
        fleet_ref = lambda c: str(t.components[c]["fleet"]).replace("{{ ghcr_org }}", ORG)
        self.assertNotIn(fleet_ref(gone), refs)
        self.assertIn(fleet_ref(kept), refs)


class LitePreset(unittest.TestCase):
    """`--preset lite` (the prepared VM image's list, goatlite.md §11 measure 5) is what a
    rendered goatlite's pins resolve to; lite.image_registry moves goat's images only."""

    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.d, True)

    def test_preset_lite_lists_what_a_rendered_goatlite_runs(self):
        table = zagros_table(self.d)
        member = run(sys.executable, IMAGES, "--member", MEMBER, "--table", table)
        preset = run(sys.executable, IMAGES, "--preset", "lite")
        self.assertEqual(member.returncode, 0, member.stderr)
        self.assertEqual(preset.returncode, 0, preset.stderr)
        self.assertTrue(preset.stdout.split())
        self.assertEqual(sorted(preset.stdout.split()), sorted(member.stdout.split()))

    def test_image_registry_moves_goat_images_and_nothing_else(self):
        reg = "registry.lan:5000/goat"
        table = zagros_table(self.d, reg)
        plain = run(sys.executable, IMAGES, "--preset", "lite").stdout.split()
        moved = run(sys.executable, IMAGES, "--preset", "lite", "--image-registry", reg).stdout.split()
        member = run(sys.executable, IMAGES, "--member", MEMBER, "--table", table).stdout.split()
        self.assertEqual(sorted(moved), sorted(member), "the rendered pins are the listed ones")
        self.assertEqual(len(plain), len(moved))
        goat = [r for r in plain if r.startswith(GOAT)]
        other = [r for r in plain if not r.startswith(GOAT)]
        self.assertTrue(goat and other, "the lite set has goat images and others (control)")
        self.assertEqual(sorted(r.replace(GOAT, reg + "/", 1) for r in goat),
                         sorted(r for r in moved if r.startswith(reg + "/")), "same name and digest")
        self.assertEqual(sorted(other), sorted(r for r in moved if not r.startswith(reg + "/")),
                         "another project's image keeps its registry")

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
