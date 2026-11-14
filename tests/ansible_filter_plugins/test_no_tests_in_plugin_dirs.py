"""No test file lives in an Ansible plugin directory.

Ansible imports every .py file in a plugin directory (ansible.cfg sets
filter_plugins = ./filter_plugins) on every playbook run, the syntax check
included. A test kept there runs as a side effect of each run, and one that
exits on a failed assertion takes the playbook run down with it. Tests for the
plugins live here, in tests/ansible_filter_plugins/.
"""

from pathlib import Path

ANSIBLE = Path(__file__).resolve().parents[2] / "ops" / "ansible"


def test_no_test_file_in_any_plugin_directory():
    found = sorted(
        str(p.relative_to(ANSIBLE))
        for d in ANSIBLE.rglob("*_plugins")
        if d.is_dir()
        for p in d.rglob("*.py")
        if p.name.startswith("test_") or p.name.endswith("_test.py")
    )
    assert found == [], "move these to tests/ansible_filter_plugins/: " + ", ".join(found)
