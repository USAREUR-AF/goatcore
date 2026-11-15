#!/usr/bin/env python3
"""Issue one acceptance-probe leaf for one run (mint-peering-leaves.sh --probe).

The leaf is a client certificate from our own peering CA that phase 14's forged-name
or impersonation probe holds (docs/design/federation-acceptance.md §2.4):
CA:FALSE, clientAuth only, digitalSignature only, a SAN URI
urn:goat:acceptance-probe:<run id> beside its DNS name, valid from a minute back to
the probe TTL plus 15 minutes. Nothing checks revocation on the federation path,
so that lifetime is the only control.

It writes <kind>-key.pem (0600) and <kind>-cert.pem into the probe folder, which
must be an empty directory of mode 0700 owned by this user, and appends one line to
the CA's issued-certificates record: run, kind, CN, serial, SHA-256, notBefore,
notAfter, when issued, and the issuer's SHA-256. The record holds certificate facts
only. Its working files (the request, the signing database) live in a private
folder inside the probe folder and are removed before it returns.
"""
import argparse
import calendar
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import stat
import subprocess
import sys
import time

MARGIN = 15 * 60
SKEW = 60


def openssl(*args):
    r = subprocess.run(['openssl', *map(str, args)], capture_output=True)
    if r.returncode != 0:
        raise SystemExit('[✗] openssl %s failed: %s' % (args[0], r.stderr.decode(errors='replace').strip()[:300]))
    return r.stdout


def utc(t):
    return time.strftime('%y%m%d%H%M%SZ', time.gmtime(t))


def x509_epoch(cert, which):
    out = openssl('x509', '-in', cert, '-noout', '-' + which).decode().split('=', 1)[1].strip()
    return calendar.timegm(time.strptime(out, '%b %d %H:%M:%S %Y GMT'))


def check_folder(out):
    st = os.lstat(out)
    if not stat.S_ISDIR(st.st_mode):
        return '%s is not a directory' % out
    if st.st_uid != os.getuid():
        return '%s is not owned by this user' % out
    if stat.S_IMODE(st.st_mode) != 0o700:
        return '%s has mode %o; a probe folder is 0700' % (out, stat.S_IMODE(st.st_mode))
    if any(Path(out).iterdir()):
        return '%s is not empty; a probe folder holds this run\'s probe identities and nothing else' % out
    return ''


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--out', required=True)
    p.add_argument('--ca-cert', required=True)
    p.add_argument('--ca-key', required=True)
    p.add_argument('--run', required=True)
    p.add_argument('--kind', required=True, choices=('forged', 'impersonate'))
    p.add_argument('--cn', required=True)
    p.add_argument('--ttl', required=True, type=int)
    p.add_argument('--issued-record', required=True)
    a = p.parse_args()

    out = Path(a.out)
    key, cert = out / (a.kind + '-key.pem'), out / (a.kind + '-cert.pem')
    work = out / '.issue'
    now = int(time.time())
    not_before, not_after = now - SKEW, now + a.ttl + MARGIN
    os.umask(0o077)
    work.mkdir(mode=0o700)
    try:
        csr = work / 'leaf.csr'
        openssl('ecparam', '-name', 'prime256v1', '-genkey', '-noout', '-out', key)
        os.chmod(key, 0o600)
        openssl('req', '-new', '-key', key, '-out', csr, '-subj', '/CN=' + a.cn)
        (work / 'index.txt').write_text('')
        (work / 'serial').write_text(secrets.token_hex(16).upper() + '\n')
        cnf = work / 'ca.cnf'
        cnf.write_text('\n'.join([
            '[ ca ]', 'default_ca = probe',
            '[ probe ]', 'database = %s/index.txt' % work, 'new_certs_dir = %s' % work,
            'serial = %s/serial' % work, 'default_md = sha256', 'policy = names', 'unique_subject = no',
            'copy_extensions = none', 'email_in_dn = no',
            '[ names ]', 'commonName = supplied',
            '[ leaf ]', 'basicConstraints = critical,CA:FALSE', 'keyUsage = critical,digitalSignature',
            'extendedKeyUsage = clientAuth',
            'subjectAltName = DNS:%s,URI:urn:goat:acceptance-probe:%s' % (a.cn, a.run), '']))
        openssl('ca', '-batch', '-notext', '-config', cnf, '-extensions', 'leaf', '-cert', a.ca_cert,
                '-keyfile', a.ca_key, '-in', csr, '-out', cert,
                '-startdate', utc(not_before), '-enddate', utc(not_after))
        os.chmod(cert, 0o644)
    except BaseException:
        for f in (key, cert):
            try:
                f.unlink()
            except FileNotFoundError:
                pass
        raise
    finally:
        shutil.rmtree(work, ignore_errors=True)

    der = openssl('x509', '-in', cert, '-outform', 'DER')
    issuer = openssl('x509', '-in', a.ca_cert, '-outform', 'DER')
    serial = openssl('x509', '-in', cert, '-noout', '-serial').decode().split('=', 1)[1].strip()
    rec = {'run': a.run, 'kind': a.kind, 'cn': a.cn, 'serial': serial,
           'sha256': hashlib.sha256(der).hexdigest(), 'not_before': x509_epoch(cert, 'startdate'),
           'not_after': x509_epoch(cert, 'enddate'), 'issued_at': now,
           'issuer_sha256': hashlib.sha256(issuer).hexdigest()}
    fd = os.open(a.issued_record, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
    with os.fdopen(fd, 'a') as f:
        f.write(json.dumps(rec, sort_keys=True) + '\n')
        f.flush()
        os.fsync(f.fileno())
    print('  [issued] %s probe leaf %s for run %s, valid until %s (serial %s), recorded in %s'
          % (a.kind, a.cn, a.run, time.strftime('%H:%M:%SZ', time.gmtime(rec['not_after'])), serial,
             a.issued_record))


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--check-folder':
        why = check_folder(sys.argv[2])
        if why:
            print(why)
            sys.exit(1)
        sys.exit(0)
    main()
