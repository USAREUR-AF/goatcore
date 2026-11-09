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
        report['observed'] = sorted(scopes)
        report['missing'] = sorted(expected - scopes)
    except (ValueError, TypeError, AttributeError):
        # Configs can hold credentials; never include their content in errors.
        report['error'] = 'Cannot inspect storage declarations in this core configuration.'
    return report


class FilterModule:
    def filters(self):
        return {'zenoh_ingress_storage_check': zenoh_ingress_storage_check}
