#!/usr/bin/env python3
"""Validate/reuse phase-8 leaves; replacement is an explicit local operation.

A rotation (--rotate) is resumable. The new pairs are minted and validated in a
private staging directory beside a copy of the pairs they replace (mode 0600),
with a journal naming the leaves; renaming that directory to
<out>/.rotation-pending is the one step that commits the rotation. The new files
are then moved into place, the installed pairs validated, and the directory
removed, so no previous pair is kept once the rotation is done. A run that finds
<out>/.rotation-pending (the one before it was killed) finishes that rotation
from what it holds and mints nothing; when the pairs it holds do not validate, it
restores the previous pairs and removes it. Either way no third pair is minted.
"""
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


def only_trust_options(ca, out):
    """`openssl verify` options that trust nothing but `ca`: no default CA file,
    path or store. OpenSSL 1.1 and later name them (-no-CAfile, -no-CApath,
    -no-CAstore). LibreSSL (macOS's openssl) has none of them and loads its
    default locations only for a -CAfile or -CApath that is not given, so both
    are given: the CA itself, and a directory that does not exist."""
    help_result = subprocess.run(['openssl', 'verify', '-help'], capture_output=True)
    usage = help_result.stderr + help_result.stdout
    if b'-no-CAfile' in usage:
        options = ['-no-CAfile', '-no-CApath']
        if b'-no-CAstore' in usage:
            options.append('-no-CAstore')
        return options
    return ['-CAfile', ca, '-CApath', out / '.no-default-ca-path']


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


STAGING_PREFIX = '.mint-'


def sweep_stale_staging(out):
    """Remove what an interrupted run left in its private staging directories.

    Every leaf is minted into <out>/.mint-*, and a run that is killed (kill -9,
    a lost host) leaves that directory behind with the private keys it had
    generated. Only this helper creates a .mint-* entry, and only while it
    holds the exclusive lock on <out>/.mint.lock; the kernel drops that lock
    when its holder dies. Called with the lock held, so every .mint-* entry
    seen here belongs to a run that is no longer alive and none is live
    material. Installed leaves, our CA, the lock file and a rotation's
    retained .previous-* set do not match the prefix and are never touched.
    A symlink or file of that name is unlinked, never followed.
    """
    for entry in sorted(out.glob(STAGING_PREFIX + '*')):
        if entry.is_dir() and not entry.is_symlink():
            shutil.rmtree(entry)
        else:
            entry.unlink()
        print('  [removed] ' + entry.name + ': staging left by an interrupted run')


OUR_CA = 'our-ca.pem'
OUR_CA_MODE = 0o600


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
    so a re-run changes nothing. Mode OUR_CA_MODE, the owner's alone, like the
    leaf keys beside it: every reader is the owner or root. The edge container
    sets no user (its image runs as root) and already reads the 0600 leaf keys
    mounted beside it; the edge role reads it on the edge with become; the
    republisher role copies it from the controller as the driver's user, who
    ran phase 8 and owns it.
    """
    target = our_ca_target(out)
    data = ca.read_bytes()
    if target.is_file() and target.read_bytes() == data:
        if stat.S_IMODE(target.stat().st_mode) != OUR_CA_MODE:
            os.chmod(target, OUR_CA_MODE)
        return
    stage = out / ('.' + OUR_CA + '.tmp')
    stage.unlink(missing_ok=True)
    fd = os.open(stage, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, OUR_CA_MODE)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(data)
    os.chmod(stage, OUR_CA_MODE)
    os.replace(stage, target)
    print('  [placed] ' + OUR_CA + ': our CA, as the agreement names it')


ROTATION = '.rotation-pending'
JOURNAL = 'journal.json'
KEY_MODE, CERT_MODE = 0o600, 0o644


def leaf_files(name):
    return (name + '.pem', name + '-key.pem')


def fsync_dir(path):
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def restore_previous(out, pending, leaves):
    """Put back the pairs the rotation replaced, at their own modes; remove a new
    file whose leaf had none before."""
    previous = pending / 'previous'
    for name, _, _ in leaves:
        for file in leaf_files(name):
            kept, target = previous / file, out / file
            if kept.is_file():
                stage = out / ('.' + file + '.restore')
                stage.unlink(missing_ok=True)
                shutil.copy2(kept, stage)
                os.chmod(stage, KEY_MODE if file.endswith('-key.pem') else CERT_MODE)
                os.replace(stage, target)
            elif target.is_file() and not target.is_symlink():
                target.unlink()


def finish_rotation(out, ca, min_valid_seconds, trust_options):
    """Finish the committed rotation in <out>/.rotation-pending: move each new file
    still there into place, validate every rotated pair, and remove the directory
    with the previous pairs in it. If a file cannot be moved or a pair does not
    validate, the previous pairs are restored and the directory removed, and the
    run is refused: nothing is minted here, whichever way it ends."""
    pending = out / ROTATION
    try:
        record = json.loads((pending / JOURNAL).read_text())
        leaves = [(str(n), str(c), str(a)) for n, c, a in record['leaves']]
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise ValueError(ROTATION + ' holds no readable journal; inspect it and its previous/ '
                         'pairs before recovery') from exc
    new = pending / 'new'
    try:
        for name, _, _ in leaves:
            for file in leaf_files(name):
                if (new / file).is_file():
                    os.replace(new / file, out / file)
        for name, cn, san in leaves:
            validate(out / (name + '.pem'), out / (name + '-key.pem'), ca, cn, san,
                     min_valid_seconds, trust_options)
    except (OSError, ValueError) as exc:
        restore_previous(out, pending, leaves)
        shutil.rmtree(pending)
        if isinstance(exc, OSError):
            raise
        raise ValueError('the rotation could not be finished (' + str(exc) + '); the previous '
                         'pairs are restored and nothing new was minted; rotate again') from exc
    shutil.rmtree(pending)
    fsync_dir(out)
    return [name for name, _, _ in leaves]


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
        journal = out / ROTATION
        if journal.is_symlink() or (journal.exists() and not journal.is_dir()):
            # An earlier release's journal file: its backup is a .previous-* set.
            raise ValueError('interrupted rotation: inspect .rotation-pending and its backup before recovery')
        sweep_stale_staging(out)
        trust_options = only_trust_options(ca, out)
        openssl('verify', *trust_options, '-trusted', ca, ca)
        our_ca_target(out)
        if journal.is_dir():
            for name in finish_rotation(out, ca, args.min_valid_seconds, trust_options):
                print('  [finished] ' + name + ': the interrupted rotation, from the pair it had minted')
            print('  previous pairs removed: the rotation is done')
            # This run's --rotate is the resumed command's: the rotation it asked
            # for is the one just finished, so nothing is minted again.
            args.rotate = False
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
        # private material stays under a private (0700) directory on the same
        # filesystem, so installation is a rename; if this run dies before the
        # rotation is committed, the next run's sweep_stale_staging removes it.
        stage = Path(tempfile.mkdtemp(prefix=STAGING_PREFIX, dir=out))
        try:
            new = stage / 'new'
            new.mkdir(mode=0o700)
            for name, cn, san in pending:
                mint(new, name, cn, san, ca, ca_key, args.days)
                validate(new / (name + '.pem'), new / (name + '-key.pem'),
                         ca, cn, san, args.min_valid_seconds, trust_options)
            if args.rotate and existing:
                previous = stage / 'previous'
                previous.mkdir(mode=0o700)
                for path in existing:
                    kept = previous / path.name
                    shutil.copy2(path, kept)
                    os.chmod(kept, KEY_MODE)
                with (stage / JOURNAL).open('x') as record:
                    json.dump({'leaves': [list(leaf) for leaf in pending]}, record)
                    record.flush()
                    os.fsync(record.fileno())
                fsync_dir(previous)
                fsync_dir(stage)
                # The commit: from here on a run finishes this rotation.
                os.rename(stage, journal)
                stage = None
                fsync_dir(out)
                finish_rotation(out, ca, args.min_valid_seconds, trust_options)
                for name, _, _ in pending:
                    print('  [issued] ' + name)
                print('  previous pairs removed: the rotation is done')
                place_our_ca(out, ca)
                return
            installed = []
            try:
                for name, _, _ in pending:
                    for file in leaf_files(name):
                        target = out / file
                        os.replace(new / file, target)
                        installed.append(target)
            except OSError:
                # A leaf that had no pair before: take back what was installed.
                for path in reversed(installed):
                    path.unlink(missing_ok=True)
                raise
            for name, _, _ in pending:
                print('  [issued] ' + name)
            place_our_ca(out, ca)
        finally:
            if stage is not None and stage.exists():
                shutil.rmtree(stage)

if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError) as error:
        raise SystemExit('[refused] ' + str(error)) from error
