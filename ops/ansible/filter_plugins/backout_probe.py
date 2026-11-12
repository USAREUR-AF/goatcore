"""What a back-out probe found: present, absent, or unknown.

The rule every probe in the peering roles' tasks/absent.yml follows
(ops/ansible/peering-backout.yml). The back-out acts on present, does nothing
for absent, and stops on unknown (goatbuilder federate backout refuses a
survey with any unknown row), so absent is given only when the first line the
tool wrote is the tool's own not-found line for the probed name:

  present  the probe ran and exited 0 (for a SPIRE bundle list: and printed a
           certificate);
  absent   docker:  exit 1 and "Error response from daemon: No such container:
                    <name>" (or "Error: No such container|object: <name>");
           podman:  exit 125 and 'Error: no such container "<name>"';
           wg:      exit 1 and "Unable to access interface: No such device";
           SPIRE:   a non-zero exit and "rpc error: code = NotFound desc = ..."
                    (with or without a leading "Error: ");
  unknown  anything else: an unreachable host, a permission error, a missing
           tool (127), a timeout, another error that mentions the same words,
           another name, another language, a probe that did not run, and a
           SPIRE bundle listed with no X.509 authority (a JWT-only bundle is
           there, but this probe cannot show it). The note carries the exit
           status and the first line of the error.

The docker and podman lines were measured (docker 29, podman 5.7) against a
container that does not exist; the SPIRE line is the server CLI's gRPC error.
"""
import re


def _first_line(text):
    for line in (text or '').splitlines():
        if line.strip():
            return line.strip()
    return ''


def _not_found(kind, rc, line, name):
    if kind == 'docker':
        if rc == 1:
            return re.fullmatch(r'Error( response from daemon)?: No such (container|object): '
                                + re.escape(name), line) is not None
        if rc == 125:
            return line == 'Error: no such container "%s"' % name
        return False
    if kind == 'wg':
        return rc == 1 and line == 'Unable to access interface: No such device'
    if kind in ('spire', 'spire-bundle'):
        return re.match(r'(Error: )?rpc error: code = NotFound desc =', line) is not None
    raise ValueError('unknown probe kind: %r' % (kind,))


def backout_probe_state(result, kind, name):
    if kind not in ('docker', 'wg', 'spire', 'spire-bundle'):
        raise ValueError('unknown probe kind: %r' % (kind,))
    if not isinstance(result, dict) or result.get('skipped') or not isinstance(result.get('rc'), int):
        return {'state': 'unknown', 'note': 'the probe did not run'}
    rc = result['rc']
    out = result.get('stdout') or ''
    err = result.get('stderr') or ''
    if rc == 0:
        if kind == 'spire-bundle' and 'BEGIN CERTIFICATE' not in out:
            return {'state': 'unknown', 'note': 'rc 0: bundle has no X.509 authority'}
        return {'state': 'present', 'note': _first_line(out)}
    line = _first_line(err) or _first_line(out)
    if _not_found(kind, rc, line, str(name)):
        return {'state': 'absent', 'note': ''}
    first = line or _first_line(result.get('msg')) or 'no error text'
    return {'state': 'unknown', 'note': 'rc %d: %s' % (rc, first[:300])}


class FilterModule:
    def filters(self):
        return {'backout_probe_state': backout_probe_state}
