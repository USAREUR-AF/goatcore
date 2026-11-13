"""Inspect receive storage using the existing endstate JSON5 parser.

This checks exact configured scopes, not delivery or peer reachability. Reusing
that parser keeps comments and ACL strings from being mistaken for stores.
"""
import importlib.util
from pathlib import Path

_PATH = Path(__file__).resolve().parents[2] / 'goatbuilder/endstate/edges.py'
_SPEC = importlib.util.spec_from_file_location('goat_ingress_endstate_edges', _PATH)
_edges = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_edges)


def _storage_scopes(text):
    """The key expression of every live storage in a core configuration's
    plugins.storage_manager.storages, read with the endstate JSON5 parser, so
    comments and ACL strings are never taken for stores. Raises ValueError."""
    section = _edges.json5_lite(text)
    if not isinstance(section, dict):
        raise ValueError('configuration must be an object')
    for name in ('plugins', 'storage_manager', 'storages'):
        section = section.get(name, {})
        if not isinstance(section, dict):
            raise ValueError('storage configuration must be an object')
    scopes = set()
    for store in section.values():
        # Null entries remove a storage in a merged config; they are no
        # evidence of a live receive subscription.
        if store is None:
            continue
        if not isinstance(store, dict):
            raise ValueError('storage entries must be objects')
        scope = store.get('key_expr')
        if scope is None:
            continue
        if not isinstance(scope, str) or not scope:
            raise ValueError('storage key expressions must be non-empty strings')
        scopes.add(scope)
    return scopes


def key_expr_includes(outer, inner):
    """Whether every key `inner` can match is also matched by `outer`, for zenoh key
    expressions: chunks between '/', '*' for exactly one chunk, '**' for any number
    of chunks. A chunk using the '$*' sub-chunk wildcard is matched only by an equal
    chunk, '*' or '**' in `outer`; in `inner` it is treated as covering any chunk.
    That can refuse an inclusion that holds, never accept one that does not."""
    def chunks(expr):
        return expr.split('/')

    def inc(g, s):
        if not g:
            return not s
        if g[0] == '**':
            return inc(g[1:], s) or (bool(s) and inc(g, s[1:]))
        if not s or s[0] == '**':
            return False
        if g[0] == '*':
            return inc(g[1:], s[1:])
        if '$*' in s[0]:
            return False
        return g[0] == s[0] and '$*' not in g[0] and inc(g[1:], s[1:])

    return inc(chunks(outer), chunks(inner))


def _may_hold_keys(scope):
    """Whether a storage's key expression can hold a key under keys/."""
    first = scope.split('/', 1)[0]
    return first in ('keys', '*', '**') or '$*' in first


def zenoh_key_storage_check(text, grants):
    """Every storage that can hold a key under keys/ must be included in one of the
    key-bundle grants: a store wider than the grant, or for another set, declares a
    queryable the edge refuses silently. {observed, wider, error}; the error never
    quotes the configuration, which can hold credentials."""
    report = {'observed': [], 'wider': [], 'error': ''}
    if not isinstance(grants, list) or not grants or any(not isinstance(g, str) or not g for g in grants):
        report['error'] = 'Provide a non-empty list of key-bundle grants.'
        return report
    try:
        scopes = sorted(scope for scope in _storage_scopes(text) if _may_hold_keys(scope))
    except (ValueError, TypeError, AttributeError):
        report['error'] = 'Cannot inspect storage declarations in this core configuration.'
        return report
    report['observed'] = scopes
    report['wider'] = [scope for scope in scopes if not any(key_expr_includes(g, scope) for g in grants)]
    return report


def zenoh_ingress_storage_check(text, required):
    report = {'observed': [], 'missing': [], 'error': ''}
    if not isinstance(required, list) or not required or any(
        not isinstance(scope, str) or not scope for scope in required
    ):
        report['error'] = 'Provide a non-empty list of admitted ingress scopes.'
        return report
    expected = set(required)
    report['missing'] = sorted(expected)
    try:
        scopes = _storage_scopes(text)
        report['observed'] = sorted(scopes)
        report['missing'] = sorted(expected - scopes)
    except (ValueError, TypeError, AttributeError):
        # Configs can hold credentials; never include their content in errors.
        report['error'] = 'Cannot inspect storage declarations in this core configuration.'
    return report


ROLE_CONFIG_PATH = '/etc/zenoh/config.json5'
# zenohd's options that take a value as the next argument: that argument is the
# option's value and never an option of its own, even when it begins with -c.
VALUE_OPTIONS = ('-l', '--listen', '-e', '--connect', '-i', '--id', '--cfg')


def zenoh_core_config_path(text):
    """Where a zenohd container reads its configuration, from its arguments.

    `text` is `docker inspect --format '{{json .Args}}'`: the arguments after
    the entrypoint, such as ["-c", "/etc/goat-core/config.json5"]. Accepted
    forms: `-c <path>`, `--config <path>`, `-c=<path>`, `-c<path>` (only when
    the path begins with `/` or `.`) and `--config=<path>`. The value after one
    of VALUE_OPTIONS is skipped. Returns {'path', 'named', 'error'}:
      - the arguments name one path (repeating it is fine): that path, named;
      - they name no configuration at all: the goat-zenoh role's path, not named;
      - anything else (unreadable arguments, a configuration argument without a
        usable value, two different paths): no path, and the error says why.
    A guess is never made: a router started with a configuration argument this
    cannot read is refused, not read at the role's path.
    """
    import json

    def refused(why):
        return {'path': '', 'named': False, 'error': why}

    try:
        args = json.loads(text)
    except (TypeError, ValueError):
        return refused('its start arguments could not be read')
    if not isinstance(args, list) or any(not isinstance(arg, str) for arg in args):
        return refused('its start arguments are not a list of strings')
    named = []
    index = 0
    while index < len(args):
        arg = args[index]
        value = None
        if arg in VALUE_OPTIONS:
            index += 2
            continue
        if arg in ('-c', '--config'):
            if index + 1 >= len(args):
                return refused(arg + ' is its last argument, with no path after it')
            value = args[index + 1]
            index += 1
        elif arg.startswith('--config='):
            value = arg[len('--config='):]
        elif arg.startswith('-c='):
            value = arg[3:]
        elif arg.startswith('-c') and not arg.startswith('--'):
            value = arg[2:]
            if not value.startswith(('/', '.')):
                return refused('cannot tell whether ' + arg + ' names a configuration: an attached'
                               ' path must begin with / or .')
        if value is not None:
            if not value:
                return refused('a configuration argument (' + arg + ') names no path')
            named.append(value)
        index += 1
    if not named:
        return {'path': ROLE_CONFIG_PATH, 'named': False, 'error': ''}
    if len(set(named)) > 1:
        return refused('its arguments name more than one configuration: ' + ', '.join(named))
    return {'path': named[0], 'named': True, 'error': ''}


class FilterModule:
    def filters(self):
        return {'zenoh_ingress_storage_check': zenoh_ingress_storage_check,
                'zenoh_key_storage_check': zenoh_key_storage_check,
                'zenoh_core_config_path': zenoh_core_config_path}
