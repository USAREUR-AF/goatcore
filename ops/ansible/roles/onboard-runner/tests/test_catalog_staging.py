"""Exercise the production staging tasks only; no credentials or hosts."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROLE = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--goat", type=Path, help="also exercise the real CLI dry-run consumer")
    binary = parser.parse_args().goat
    if binary:
        binary = binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="runner-catalog-") as directory:
        root = Path(directory)
        source = root / "catalogs"
        source.mkdir()
        repo = root / "repo"
        profile = root / "profile"
        context = profile / "contexts/default"
        context.mkdir(parents=True)
        (context / "profile.toml").write_text('portal_endpoint = "http://127.0.0.1:9"\n')
        (context / "portal.token").write_text("fixture-token")
        manifest = repo / "ops/operators/fixturenet/fixture.yml"
        manifest.parent.mkdir(parents=True)
        manifest_text = ('identity: {handle: fixture, email: fixture@example.invalid}\n'
                         'org: {short_tag: fixture-org}\n'
                         'affiliation: {id: fixture-org}\n'
                         'netbird: {enabled: false}\nzenoh: {enabled: false}\n'
                         'ory_site: {enabled: true}\n')
        manifest.write_text(manifest_text)
        play = root / "play.json"
        play.write_text(json.dumps([{"hosts": "localhost", "gather_facts": False,
            "tasks": [{"ansible.builtin.include_tasks": str(ROLE / "tasks/affiliation-catalog.yml")}]}]))
        variables = {"onboard_runner_aff_trait_enabled": True,
                     "onboard_runner_only_env": "fixturenet",
                     "onboard_runner_catalog_dir_src": str(source),
                     "onboard_runner_repo_root_host": str(repo),
                     "onboard_runner_uid": os.getuid(),
                     # The role has no default image; a deploy takes it from pins.
                     "onboard_runner_image": "registry.example/onboard-runner@sha256:" + "0" * 64,
                     "ansible_python_interpreter": sys.executable}

        def stage(label, *, enabled=True, environment="fixturenet", success=True):
            values = {**variables, "onboard_runner_aff_trait_enabled": enabled,
                      "onboard_runner_only_env": environment}
            result = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local",
                                     str(play), "-e", json.dumps(values)],
                                    capture_output=True, text=True, timeout=90)
            assert (result.returncode == 0) == success, result.stdout + result.stderr
            print(label + ": " + ("PASS" if success else "expected refusal"), flush=True)
            return result.stdout

        def rendered_emission(enabled):
            # Render the actual production consumer with role defaults. Extra
            # vars model CLI -e values, including their common string forms.
            compose = root / "compose.yaml"
            render_play = root / "render.json"
            render_play.write_text(json.dumps([{
                "hosts": "localhost", "gather_facts": False,
                "vars_files": [str(ROLE / "defaults/main.yml")],
                "tasks": [{"ansible.builtin.template": {
                    "src": str(ROLE / "templates/docker-compose.yml.j2"),
                    "dest": str(compose)}}]}]))
            values = {**variables, "site_id": "fixture", "ghcr_org": "fixture",
                      "public_mgmt_fqdn": "netbird.example.invalid",
                      "onboard_runner_mtls_enabled": False,
                      "onboard_runner_aff_trait_enabled": enabled}
            result = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local",
                                     str(render_play), "-e", json.dumps(values)],
                                    capture_output=True, text=True, timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            return bool(re.search(r"^\s+GOAT_ORY_AFF_TRAIT_ENABLED:",
                                  compose.read_text(), re.MULTILINE))

        def consume(success, enabled=True):
            if not binary:
                return
            before = manifest.read_bytes()
            result = subprocess.run([str(binary), "--profile-dir", str(profile), "admin", "onboard",
                                     "fixture", "--native", "--env", "fixturenet", "--repo-root", str(repo),
                                     "--dry-run"], capture_output=True, text=True, timeout=30,
                                    env={"PATH": os.environ["PATH"], "HOME": str(root),
                                         "GOAT_OPERATOR_HANDLE": "onboard-runner-fixture",
                                         "GOAT_ORY_AFF_TRAIT_ENABLED": str(enabled).lower()})
            assert (result.returncode == 0) == success, result.stdout + result.stderr
            if not success:
                assert "read affiliation catalog" in result.stderr, result.stderr
            assert manifest.read_bytes() == before
            assert not (repo / "ops/operators/out/fixture-bundle.md").exists()
            print("CLI deployed-layout dry run: " + ("PASS" if success else "expected missing-catalog refusal"), flush=True)

        for flag in [False, "false", "0", "off", "no"]:
            stage("disabled without catalog: " + repr(flag), enabled=flag)
            emitted = rendered_emission(flag)
            consume(True, enabled=emitted)
            assert not emitted, "disabled flag enabled CLI emission without staging: " + repr(flag)
        manifest.write_text(manifest_text.replace("affiliation: {id: fixture-org}",
                                                  "affiliation: {id: fixture-org, kind: partner}"))
        consume(True)
        manifest.write_text(manifest_text)
        consume(False)
        stage("disabled without catalog", enabled=False)
        stage("enabled without catalog", success=False)
        stage("invalid environment", environment="../outside", success=False)
        foreign = source / "other.yaml"
        foreign.write_text("orgs: [{id: foreign, sector: commercial}]\n")
        stage("another environment is not a fallback", success=False)
        yaml_source = source / "fixturenet.yaml"
        yaml_source.write_text("orgs: [{id: fixture-org, sector: commercial}]\n")
        stage("fresh YAML catalog")
        assert rendered_emission(True)
        assert rendered_emission("true")
        dest = repo / "ops/identity/orgs"
        assert (dest / "fixturenet.yaml").read_bytes() == yaml_source.read_bytes()
        assert not (dest / "other.yaml").exists()
        for path in [repo, repo / "ops", repo / "ops/identity", dest]:
            assert path.stat().st_uid == os.getuid()
            assert path.stat().st_mode & 0o777 == 0o750
        assert (dest / "fixturenet.yaml").stat().st_mode & 0o777 == 0o644
        consume(True)
        assert re.search(r"changed=0\b", stage("unchanged convergence"))
        yaml_source.write_text("orgs: [{id: fixture-org, sector: academic}]\n")
        stage("catalog refresh")
        assert (dest / "fixturenet.yaml").read_bytes() == yaml_source.read_bytes()
        json_source = source / "fixturenet.json"
        json_source.write_text(json.dumps({"orgs": [{"id": "fixture-org", "sector": "commercial"}]}))
        stage("JSON precedence")
        assert (dest / "fixturenet.json").read_bytes() == json_source.read_bytes()
        assert not (dest / "fixturenet.yaml").exists()
        consume(True)
        json_source.unlink()
        stage("switch back to YAML removes stale JSON")
        assert not (dest / "fixturenet.json").exists()
        assert (dest / "fixturenet.yaml").read_bytes() == yaml_source.read_bytes()
        consume(True)
        yaml_source.unlink()
        stage("missing source refuses without discarding prior catalog", success=False)
        assert (dest / "fixturenet.yaml").is_file()
    print("Runner catalog staging controls passed.")


if __name__ == "__main__":
    main()
