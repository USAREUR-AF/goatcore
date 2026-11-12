"""What a back-out probe found: present, absent, or unknown.

The rule every probe in the peering roles' tasks/absent.yml follows
(ops/ansible/peering-backout.yml). The back-out acts on present, does nothing
for absent, and stops on unknown (goatbuilder federate backout refuses a
survey with any unknown row), so absent is given only when the tool said, in
its own words, that the resource does not exist:

  present  the probe ran and exited 0 (for a SPIRE bundle list: and printed a
           certificate);
  absent   docker or podman: exit 1 with "No such container" or "No such
           object"; wg: exit 1 with "No such device"; the SPIRE server CLI:
           a non-zero exit with "code = NotFound"; a SPIRE bundle list: exit 0
           with no certificate;
  unknown  anything else: an unreachable host, a permission error, a missing
           tool (127), a timeout, a probe that did not run. The note carries
           the exit status and the first line of the error.
"""
import re

_NOT_FOUND = {
    'docker': re.compile(r'no such (container|object)', re.I),
    'wg': re.compile(r'No such device'),
    'spire': re.compile(r'code = NotFound'),
    'spire-bundle': re.compile(r'code = NotFound'),
}


def backout_probe_state(result, kind):
    if kind not in _NOT_FOUND:
        raise ValueError('unknown probe kind: %r' % (kind,))
    if not isinstance(result, dict) or result.get('skipped') or not isinstance(result.get('rc'), int):
        return {'state': 'unknown', 'note': 'the probe did not run'}
    rc = result['rc']
    out = result.get('stdout') or ''
    err = result.get('stderr') or ''
    if rc == 0:
        if kind == 'spire-bundle' and 'BEGIN CERTIFICATE' not in out:
            return {'state': 'absent', 'note': ''}
        return {'state': 'present', 'note': out.strip().splitlines()[0] if out.strip() else ''}
    definite = rc == 1 if kind in ('docker', 'wg') else True
    if definite and _NOT_FOUND[kind].search(err + '\n' + out):
        return {'state': 'absent', 'note': ''}
    first = (err.strip() or out.strip() or result.get('msg') or 'no error text').splitlines()[0]
    return {'state': 'unknown', 'note': 'rc %d: %s' % (rc, first[:300])}


class FilterModule:
    def filters(self):
        return {'backout_probe_state': backout_probe_state}
