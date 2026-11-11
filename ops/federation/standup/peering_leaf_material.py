#!/usr/bin/env python3
"""Validate/reuse phase-8 leaves; replacement is an explicit local operation."""
import argparse
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile


def openssl(*args, data=None):
    result = subprocess.run(['openssl', *map(str, args)], input=data,
                            capture_output=True, env={**os.environ, 'LC_ALL': 'C'})
    if result.returncode:
        raise ValueError('OpenSSL rejected certificate or key material')
    return result.stdout


def public_key(path, certificate=False):
    if certificate:
        pem = openssl('x509', '-in', path, '-pubkey', '-noout')
        return openssl('pkey', '-pubin', '-outform', 'DER', data=pem)
    return openssl('pkey', '-in', path, '-passin', 'pass:', '-pubout', '-outform', 'DER')


def extension(text, name):
    match = re.search(r'X509v3 ' + re.escape(name) + r':[^\n]*\n\s*([^\n]+)', text)
    if not match:
        raise ValueError('missing ' + name)
    return match[1].strip()


def sans(value):
    result = set()
    for item in value.split(','):
        kind, separator, name = item.strip().partition(':')
        if not separator:
            raise ValueError('invalid subject alternative name')
        if kind in ('IP', 'IP Address'):
            result.add(('IP', str(ipaddress.ip_address(name.strip()))))
        elif kind == 'DNS' and re.fullmatch(r'[A-Za-z0-9_.-]+', name):
            result.add(('DNS', name.lower()))
        else:
            raise ValueError('unsupported subject alternative name')
    return result


def validate(cert, key, ca, cn, expected_sans, min_seconds, trust_options):
    if stat.S_IMODE(key.stat().st_mode) != 0o600:
        raise ValueError('private key must have mode 0600')
    if public_key(cert, certificate=True) != public_key(key):
        raise ValueError('certificate and private key do not match')
    subject = openssl('x509', '-in', cert, '-subject', '-noout',
                      '-nameopt', 'RFC2253').decode().strip()
    if subject.partition('=')[2].strip() != 'CN=' + cn:
        raise ValueError('certificate subject differs from the requested identity')
    text = openssl('x509', '-in', cert, '-text', '-noout').decode()
    if sans(extension(text, 'Subject Alternative Name')) != sans(expected_sans):
        raise ValueError('certificate SANs differ from the requested addresses')
    if extension(text, 'Basic Constraints') != 'CA:FALSE':
        raise ValueError('expected a non-CA leaf')
    expected_eku = {'TLS Web Server Authentication', 'TLS Web Client Authentication'}
    if set(extension(text, 'Extended Key Usage').split(', ')) != expected_eku:
        raise ValueError('unexpected extended key usage')
    if set(extension(text, 'Key Usage').split(', ')) != {'Digital Signature', 'Key Encipherment'}:
        raise ValueError('unexpected key usage')
    for purpose in ('sslserver', 'sslclient'):
        openssl('verify', *trust_options, '-trusted', ca, '-purpose', purpose, cert)
    openssl('x509', '-in', cert, '-noout', '-checkend', min_seconds)


def mint(folder, name, cn, san, ca, ca_key, days):
    key, cert = folder / (name + '-key.pem'), folder / (name + '.pem')
    openssl('ecparam', '-name', 'prime256v1', '-genkey', '-noout', '-out', key)
    key.chmod(0o600)
    csr, ext = folder / (name + '.csr'), folder / (name + '.ext')
    openssl('req', '-new', '-key', key, '-out', csr, '-subj', '/CN=' + cn)
    ext.write_text('subjectAltName=' + san + '\nbasicConstraints=CA:FALSE\n'
                   'keyUsage=digitalSignature,keyEncipherment\n'
                   'extendedKeyUsage=serverAuth,clientAuth\n')
    # Random serials avoid shared CA serial-file mutation, including concurrent
    # runs using the same offline CA in different output directories.
    serial = hex(int.from_bytes(os.urandom(19), 'big') or 1)
    openssl('x509', '-req', '-in', csr, '-CA', ca, '-CAkey', ca_key,
            '-passin', 'pass:', '-set_serial', serial, '-out', cert,
            '-days', days, '-sha256', '-extfile', ext)
    cert.chmod(0o644)


OUR_CA = 'our-ca.pem'


def our_ca_target(out):
    """Where our CA goes beside the leaves: refused, before anything is issued,
    when something other than a regular file holds that path."""
    target = out / OUR_CA
    if target.is_symlink() or (target.exists() and not target.is_file()):
        raise ValueError(OUR_CA + ': expected a regular file at ' + str(target)
                         + '; a container started before phase 8 creates a directory at a '
                         'mounted path that does not exist. Remove it and run phase 8 again')
    return target


def place_our_ca(out, ca):
    """Our CA, the one the agreement names for us, beside the leaves it signs.

    The rendered deployment halves name <out>/our-ca.pem as the CA of the
    edge's inward endpoint (root_ca_certificate_file) and as the republisher's
    core CA, and the edge mounts that path into its container. Nothing else
    writes it, and a bind mount of a path that does not exist makes the
    container runtime create a directory there: zenoh then cannot read the CA,
    discards the endpoint and never dials the core, with nothing logged.
    Written once the leaves are in place, and only when the content differs,
    so a re-run changes nothing.
    """
    target = our_ca_target(out)
    data = ca.read_bytes()
    if target.is_file() and target.read_bytes() == data:
        return
    stage = out / ('.' + OUR_CA + '.tmp')
    stage.unlink(missing_ok=True)
    fd = os.open(stage, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(data)
    os.chmod(stage, 0o644)
    os.replace(stage, target)
    print('  [placed] ' + OUR_CA + ': our CA, as the agreement names it')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--ca-cert', required=True, type=Path)
    parser.add_argument('--ca-key', type=Path)
    parser.add_argument('--ca-fingerprint', required=True)
    parser.add_argument('--days', type=int, default=90)
    parser.add_argument('--min-valid-seconds', type=int, default=86400)
    parser.add_argument('--rotate', action='store_true')
    parser.add_argument('--leaf', action='append', nargs=3, required=True)
    args = parser.parse_args()
    if args.days <= 0 or args.min_valid_seconds < 0:
        parser.error('days must be positive and minimum validity non-negative')
    names = [name for name, _, _ in args.leaf]
    if len(names) != len(set(names)):
        parser.error('duplicate leaf name')
    for name, cn, san in args.leaf:
        if not re.fullmatch(r'[a-z][a-z-]*', name) or not re.fullmatch(r'[A-Za-z0-9_.-]+', cn):
            parser.error('invalid leaf name or common name')
        sans(san)
    out = args.out.absolute()
    ca = args.ca_cert.absolute()
    if ca.read_text().count('-----BEGIN CERTIFICATE-----') != 1:
        raise ValueError('supply exactly one own CA certificate, not a trust bundle')
    fingerprint = 'sha256:' + hashlib.sha256(openssl('x509', '-in', ca, '-outform', 'DER')).hexdigest()
    if fingerprint != args.ca_fingerprint:
        raise ValueError('own CA fingerprint differs from the agreement')
    if out.is_symlink():
        raise ValueError('output directory must not be a symlink')
    out.mkdir(mode=0o700, parents=True, exist_ok=True)
    # Lock before inspection and retain it through installation. No stale PID
    # deletion or stealing: flock is released by the kernel when the process exits.
    lock_path = out / '.mint.lock'
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ValueError('another certificate operation holds this output directory') from exc
        journal = out / '.rotation-pending'
        if journal.exists() or journal.is_symlink():
            raise ValueError('interrupted rotation: inspect .rotation-pending and its backup before recovery')
        help_result = subprocess.run(['openssl', 'verify', '-help'], capture_output=True)
        trust_options = ['-no-CAfile', '-no-CApath']
        if b'-no-CAstore' in help_result.stderr + help_result.stdout:
            trust_options.append('-no-CAstore')
        openssl('verify', *trust_options, '-trusted', ca, ca)
        our_ca_target(out)
        pending = []
        existing = []
        for name, cn, san in args.leaf:
            cert, key = out / (name + '.pem'), out / (name + '-key.pem')
            for path in (cert, key):
                if path.is_symlink() or (path.exists() and not path.is_file()):
                    raise ValueError(name + ': expected regular files, refusing replacement')
                if path.exists():
                    existing.append(path)
            if cert.exists() != key.exists() and not args.rotate:
                raise ValueError(name + ': incomplete pair; inspect it, then explicitly rotate if intended')
            if cert.exists() and key.exists() and not args.rotate:
                try:
                    validate(cert, key, ca, cn, san, args.min_valid_seconds, trust_options)
                except ValueError as exc:
                    raise ValueError(name + ': ' + str(exc) + '; inspect it before using --rotate') from exc
            else:
                pending.append((name, cn, san))
        if not pending:
            for name in names:
                print('  [reused] ' + name + ': existing certificate and key validated')
            place_our_ca(out, ca)
            return
        if args.ca_key is None or not args.ca_key.is_file():
            raise ValueError('issuance requires our CA key; set FED_CA_KEY or --ca-key, '
                             'or opt in to the age-encrypted key with FED_CA_KEY_FROM_AGE=1 '
                             '(FED_CA_KEY_AGE with FED_AGE_IDENTITY)')
        ca_key = args.ca_key.absolute()
        if public_key(ca, certificate=True) != public_key(ca_key):
            raise ValueError('signing CA certificate and private key do not match')
        # Validate the complete candidate set before replacing any leaf. Temporary
        # private material stays under a private directory on the same filesystem.
        with tempfile.TemporaryDirectory(prefix='.mint-', dir=out) as temp:
            stage = Path(temp)
            for name, cn, san in pending:
                mint(stage, name, cn, san, ca, ca_key, args.days)
                validate(stage / (name + '.pem'), stage / (name + '-key.pem'),
                         ca, cn, san, args.min_valid_seconds, trust_options)
            backup = None
            if args.rotate and existing:
                backup = Path(tempfile.mkdtemp(prefix='.previous-', dir=out))
                for path in existing:
                    shutil.copy2(path, backup / path.name)
            if backup:
                with journal.open('x') as record:
                    json.dump({'backup': backup.name, 'leaves': names}, record)
                    record.flush()
                    os.fsync(record.fileno())
            installed = []
            try:
                for name, _, _ in pending:
                    for suffix in ('.pem', '-key.pem'):
                        target = out / (name + suffix)
                        os.replace(stage / target.name, target)
                        installed.append(target)
            except OSError:
                # Best-effort rollback on an ordinary installation error. An
                # abrupt host/process failure may require the retained backup;
                # the next run validates pairs and refuses inconsistent material.
                for path in reversed(installed):
                    if backup and (backup / path.name).exists():
                        shutil.copy2(backup / path.name, path)
                    elif path not in existing:
                        path.unlink(missing_ok=True)
                if backup:
                    journal.unlink()
                raise
            if backup:
                journal.unlink()
            for name, _, _ in pending:
                print('  [issued] ' + name)
            place_our_ca(out, ca)
            if backup:
                print('  Previous material retained at ' + str(backup))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError) as error:
        raise SystemExit('[refused] ' + str(error)) from error
