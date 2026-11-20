"""The CGNAT validation runbook and design document name only files that
exist, and the runbook's commands put no secret value on a
command line.

A path counts when it is in backticks or a Markdown link and looks like a file
in the repository (it has a slash or a file extension). It resolves against the
document's own directory, or against the repository root. Placeholders in angle
brackets are skipped.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
RUNBOOK = ROOT / 'docs/operations/cgnat-validation-runbook.md'
DESIGN = ROOT / 'docs/design/cgnat-validation-env.md'

PATHLIKE = re.compile(r'^[\w./-]+(?:/[\w./-]*|\.(?:md|yml|yaml|sh|tf|ini|go|py))$')


def named_paths(text):
    found = set(re.findall(r'`([^`\n]+)`', text)) | set(re.findall(r'\]\(([^)\s#]+)', text))
    return sorted(p for p in found if '<' not in p and PATHLIKE.match(p))


def missing(doc, text):
    return [p for p in named_paths(text)
            if not ((doc.parent / p).exists() or (ROOT / p.lstrip('./')).exists())]


def section(text, heading):
    m = re.search(rf'^## {re.escape(heading)}\n(.*?)(?=^## |\Z)', text, re.M | re.S)
    if m is None:
        raise AssertionError(f'no section "## {heading}"')
    return m.group(1)


class CgnatValidationDocs(unittest.TestCase):

    def test_runbook_names_only_files_that_exist(self):
        text = RUNBOOK.read_text()
        self.assertEqual(missing(RUNBOOK, text), [])
        # Commands too: a repository path a command runs or reads.
        blocks = '\n'.join(re.findall(r'```bash\n(.*?)```', text, re.S))
        run = sorted(set(re.findall(r'(?<![\w/.])((?:ops|docs|terraform|signal-server|crates)/[\w./-]+)', blocks)))
        self.assertEqual([p for p in run if not (ROOT / p).exists()], [])

    def test_design_artifact_list_names_only_files_that_exist(self):
        text = section(DESIGN.read_text(), '9. Related artifacts')
        self.assertTrue(named_paths(text), 'the artifact list names nothing')
        self.assertEqual(missing(DESIGN, text), [])

    def test_design_document_names_only_files_that_exist(self):
        self.assertEqual(missing(DESIGN, DESIGN.read_text()), [])

    def test_runbook_commands_put_no_secret_on_a_command_line(self):
        blocks = '\n'.join(re.findall(r'```bash\n(.*?)```', RUNBOOK.read_text(), re.S))
        # -e name="$VALUE" or -e name=$(...) expands a value into the command line;
        # a play reads its own environment with lookup("env", ...) instead.
        self.assertEqual(re.findall(r'-e [a-z_]+=["\']?\$', blocks), [])
        self.assertNotIn('PGPASSWORD=', blocks)
        self.assertIn('lookup("env"', blocks)


if __name__ == '__main__':
    unittest.main()
