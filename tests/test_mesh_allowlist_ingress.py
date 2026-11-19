"""The mesh-only allowlist trusts the mgmt Docker network; nothing off-host may arrive from it.

mesh-source-allowlist (mgmt-dynamic.yml.j2) admits the mesh range and the mgmt
Docker network's own subnet (mgmt_docker_network_subnet), for on-host
containers. Docker's userland proxy reconnects from that network's gateway, so
any off-host client it carries also arrives from that subnet and passes the
gate. Two paths lead there:

1. A Traefik publish that listens on [::] on an IPv4-only network: docker-proxy
   serves it, and every IPv6 LAN client becomes 172.x.0.1 (GHSA-p22f).
2. A host-local forwarder that relays off-host traffic to websecure: the
   rathole client delivering public relay traffic to 127.0.0.1:443.

3. An allowlist wider than the mesh range plus that subnet (the whole
   172.16.0.0/12 admitted any off-host client numbered inside it), or one that
   judges X-Forwarded-For (ipStrategy) or a PROXY header instead of the peer.

These tests fail if the traefik publish shape, an allowlist, Traefik's static
trust settings, a router's entrypoint, a rathole target or a live row reverts.
Run: python3 -m unittest discover -s tests -p test_mesh_allowlist_ingress.py
"""
import ipaddress
from pathlib import Path
import re
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'ops/compose/mgmt-host/docker-compose.yml'
DYNAMIC = ROOT / 'ops/ansible/templates/traefik-acme/mgmt-dynamic.yml.j2'
ENROLLMENT = '127.0.0.1:8443'
DEFAULTED = re.compile(r'^\$\{[A-Z0-9_]+:-([^}]*)\}$')


def compose_files():
    # Every YAML file beside the base compose: an overlay need not be named docker-compose*.
    files = sorted(p for p in (ROOT / 'ops/compose/mgmt-host').iterdir()
                   if p.is_file() and re.search(r'\.ya?ml(\.j2)?$', p.name))
    files += sorted((ROOT / 'ops/ansible/templates').glob('*/docker-compose*.j2'))
    return files


def load_compose(path):
    """A compose file with every Jinja branch kept, whatever its suffix: a plain
    .yml may carry {{ }} too (it is templated by the role that ships it)."""
    text = path.read_text()
    if '{{' in text or '{%' in text or '{#' in text:
        text, _ = flatten_text(text)
        return load_merged(text) or {}
    return yaml.safe_load(text) or {}


def split_short(entry):
    """Split a short-syntax publish into (host_ip, host_port, container_port)."""
    spec = entry.split('/')[0]
    if spec.startswith('['):
        host, _, rest = spec[1:].partition(']')
        parts = rest.lstrip(':').split(':')
        return host, parts[0], parts[1] if len(parts) > 1 else parts[0]
    parts = spec.split(':')
    if spec.startswith('${'):  # ${VAR:-default}:host:container
        end = spec.index('}') + 1
        parts = [spec[:end]] + spec[end + 1:].split(':')
    if len(parts) == 1:
        return '', parts[0], parts[0]
    if len(parts) == 2:
        return '', parts[0], parts[1]
    if len(parts) == 3:
        return parts[0], parts[1], parts[2]
    return ':'.join(parts[:-2]), parts[-2], parts[-1]  # bare IPv6 host address


def publish_problem(entry):
    """Why a traefik publish could hand an off-host client to docker-proxy, or None."""
    if isinstance(entry, dict):
        host, target = str(entry.get('host_ip', '')), str(entry.get('target', ''))
    else:
        host, _, target = split_short(str(entry))
    default = DEFAULTED.match(host)
    if default:
        host = default.group(1)
    if not host:
        return 'no host address: Docker also listens on [::] through docker-proxy'
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        return f'host address {host!r} is not an IPv4 literal'
    if address.version != 4:
        return f'IPv6 host address [{host}]: docker-proxy SNATs it to the bridge gateway'
    if target.split('-')[0] == '443' and host != '0.0.0.0':
        return f'websecure must be published as 0.0.0.0:443:443, got host {host}'
    return None


def publishes_port(entry, port):
    if isinstance(entry, dict):
        return port in (str(entry.get('target', '')), str(entry.get('published', '')))
    _, host_port, container_port = split_short(str(entry))
    return port in (host_port.split('-')[0], container_port.split('-')[0])


def static_trust_problems(args, env, mesh_eps):
    """Static Traefik settings that let a client choose the source the allowlist sees."""
    out = []
    for arg in args:
        low = arg.lower()
        if low.startswith('--providers.docker'):
            out.append(f'{arg}: label-defined routers escape every router guard here')
        m = re.match(r'--entrypoints\.([^.=]+)\.(proxyprotocol\.|forwardedheaders\.insecure)', low)
        if m and m.group(1) in {e.lower() for e in mesh_eps}:
            out.append(f'{arg}: a mesh-only entrypoint must judge the TCP peer, not a client header')
    for item in env:
        key = item.split('=', 1)[0].upper()
        if key.startswith('TRAEFIK_PROVIDERS_DOCKER'):
            out.append(f'{item}: label-defined routers escape every router guard here')
        for ep in mesh_eps:
            prefix = f'TRAEFIK_ENTRYPOINTS_{ep.upper()}_'
            if key.startswith(prefix + 'PROXYPROTOCOL') or key.startswith(prefix + 'FORWARDEDHEADERS_INSECURE'):
                out.append(f'{item}: a mesh-only entrypoint must judge the TCP peer, not a client header')
    return out


def toml_local_addr(text, service='netbird-mgmt'):
    block = re.search(r'\[client\.services\.' + re.escape(service) + r'\][^\[]*', text)
    return re.search(r'local_addr\s*=\s*"([^"]+)"', block.group(0)).group(1)


class PublishShape(unittest.TestCase):
    def test_classifier_rejects_every_dual_stack_or_ipv6_form(self):
        for bad in ('443:443', '443', '[::]:443:443', '::::443:443', '${BIND:-}:443:443',
                    '${BIND:-::}:8082:8082', {'target': 443, 'published': 443},
                    {'target': 443, 'published': 443, 'host_ip': '::'},
                    '127.0.0.1:443:443', 'example.test:443:443'):
            self.assertIsNotNone(publish_problem(bad), bad)
        for good in ('0.0.0.0:443:443', '127.0.0.1:8443:8443', '${TRAEFIK_METRICS_BIND:-127.0.0.1}:8082:8082',
                     {'target': 443, 'published': 443, 'host_ip': '0.0.0.0'}):
            self.assertIsNone(publish_problem(good), good)

    def test_traefik_publishes_ipv4_only_in_every_compose_file(self):
        seen = 0
        for path in compose_files():
            services = load_compose(path).get('services') or {}
            traefik = services.get('traefik') or {}
            # host or service:<x>: Traefik would inherit someone else's sockets.
            self.assertNotIn('network_mode', traefik, path)
            for entry in traefik.get('ports') or []:
                seen += 1
                self.assertIsNone(publish_problem(entry), f'{path.relative_to(ROOT)}: {entry!r}')
            for name, svc in services.items():  # nothing else may take :443 either
                for entry in (svc or {}).get('ports') or []:
                    if name != 'traefik' and publishes_port(entry, '443'):
                        self.fail(f'{path.relative_to(ROOT)}: service {name} publishes {entry!r}')
        self.assertGreater(seen, 0, 'no traefik publish found; the guard is reading the wrong files')

    def test_traefik_static_config_trusts_no_client_supplied_source(self):
        mesh_eps = mesh_only_entrypoints()
        self.assertIn('websecure', mesh_eps)
        seen = 0
        for path in compose_files():
            traefik = (load_compose(path).get('services') or {}).get('traefik') or {}
            args = [str(a) for a in traefik.get('command') or []]
            env = traefik.get('environment') or {}
            env = [f'{k}={v}' for k, v in env.items()] if isinstance(env, dict) else [str(e) for e in env]
            seen += len(args)
            for problem in static_trust_problems(args, env, mesh_eps):
                self.fail(f'{path.relative_to(ROOT)}: {problem}')
        self.assertGreater(seen, 20, 'no traefik command found; the guard is reading the wrong files')

    def test_static_trust_classifier(self):
        eps = {'websecure'}
        for bad in ('--entrypoints.websecure.proxyProtocol.insecure=true',
                    '--entrypoints.websecure.proxyprotocol.trustedIPs=10.0.0.0/8',
                    '--entryPoints.websecure.forwardedHeaders.insecure=true',
                    '--providers.docker=true', '--providers.docker.exposedByDefault=false'):
            self.assertTrue(static_trust_problems([bad], [], eps), bad)
        self.assertTrue(static_trust_problems([], ['TRAEFIK_ENTRYPOINTS_WEBSECURE_PROXYPROTOCOL_INSECURE=true'], eps))
        self.assertTrue(static_trust_problems([], ['TRAEFIK_PROVIDERS_DOCKER=true'], eps))
        for good in ('--entrypoints.enrollment.proxyProtocol.insecure=true',
                     '--entrypoints.websecure.forwardedHeaders.trustedIPs=127.0.0.1/32',
                     '--providers.file.directory=/etc/traefik/dynamic'):
            self.assertFalse(static_trust_problems([good], [], eps), good)

    def test_mgmt_network_is_not_ipam_pinned(self):
        # The allowlist subnet is read from the live network. An ipam block would
        # make Compose recreate the network on rollout and strand the stack.
        net = load_compose(BASE)['networks']['mgmt']
        self.assertEqual(net.get('name'), 'mgmt')
        self.assertNotIn('ipam', net)
        task = (ROOT / 'ops/ansible/tasks/mgmt-network-subnet.yml').read_text()
        self.assertIn('[docker, network, inspect, mgmt', task)
        for play in ('ops/ansible/traefik-acme.yml', 'ops/ansible/lan-traefik-routes.yml',
                     'ops/ansible/roles/icam-mtls-proxy/tasks/main.yml'):
            text = (ROOT / play).read_text()
            self.assertIn('tasks/mgmt-network-subnet.yml', text, play)
            render = {'ops/ansible/traefik-acme.yml': 'src: traefik-acme/mgmt-dynamic.yml.j2',
                      'ops/ansible/lan-traefik-routes.yml': 'src: templates/lan-traefik/lan-mgmt-dynamic.yml.j2',
                      'ops/ansible/roles/icam-mtls-proxy/tasks/main.yml': 'src: traefik-passthrough.yml.j2'}[play]
            self.assertLess(text.index('tasks/mgmt-network-subnet.yml'), text.index(render),
                            f'{play}: the subnet must be read before the allowlist is rendered')

    def test_subnet_task_refuses_a_wide_or_foreign_range(self):
        task = yaml.safe_load((ROOT / 'ops/ansible/tasks/mgmt-network-subnet.yml').read_text())
        check = next(t for t in task if 'ansible.builtin.assert' in t)['ansible.builtin.assert']['that']
        pattern = re.fullmatch(r"mgmt_docker_network_subnet is match\('(.+)'\)", check[0].strip()).group(1)
        for bad in ('', '172.16.0.0/12', '172.0.0.0/8', '10.0.0.0/8', '0.0.0.0/0', '100.64.0.0/10',
                    '172.32.0.0/16', '192.168.0.0/15', '172.21.0.0/16, 0.0.0.0/0'):
            self.assertIsNone(re.match(pattern, bad), bad)
        for good in ('172.21.0.0/16', '172.31.250.0/24', '192.168.16.0/20'):
            self.assertIsNotNone(re.match(pattern, good), good)

    def test_base_publish_is_websecure_ipv4_and_enrollment_loopback(self):
        ports = load_compose(BASE)['services']['traefik']['ports']
        self.assertIn('0.0.0.0:443:443', ports)
        self.assertIn(ENROLLMENT + ':8443', ports)

    def test_allowlist_trusts_bridge_only_with_its_condition_stated(self):
        text = DYNAMIC.read_text()
        block = text[text.index('    mesh-source-allowlist:'):]
        block = block[:block.index('{% for cidr in mesh_only_allowlist_extra_cidrs')]
        self.assertIn('{{ mgmt_docker_network_subnet }}', block)
        self.assertNotIn('"172.16.0.0/12"', block)
        self.assertNotIn('no off-host client arrives through docker-proxy', block)
        self.assertIn(ENROLLMENT, block)


def _is_dynamic(path):
    if not path.is_file() or not re.search(r'\.(j2|ya?ml)$', path.name):
        return False
    text = path.read_text(errors='replace')
    return bool(re.search(r'^\s*(routers|middlewares):\s*$', text, re.M)
                and re.search(r'^(http|tcp|udp):\s*$', text, re.M))


# Every Traefik dynamic file that Ansible renders or copies, or compose mounts:
# templates and plain .yml alike (a role's files/ is delivered with copy:).
ROUTER_TEMPLATES = sorted(p for tree in ('ops/ansible', 'ops/compose')
                          for p in (ROOT / tree).rglob('*') if _is_dynamic(p))

# Routers whose entryPoints is a Jinja variable. Each must be listed here, and is
# checked under its role default AND every value any ops/ansible file assigns it.
TEMPLATED_ENTRYPOINTS = {
    ('roles/onboard-intake-collector/templates/traefik-onboard-intake.yml.j2', 'onboard-intake'):
        'onboard_intake_entrypoint',
    ('roles/icam-mtls-proxy/templates/traefik-passthrough.yml.j2', 'icam-mtls-proxy'):
        'icam_mtls_proxy_traefik_entrypoint',
}
JINJA_VAR = re.compile(r'^\s*([a-z_][a-z0-9_]*)\s*$')


TAG = re.compile(r'\{%(-?)\s*(\w+)(.*?)\s*(-?)%\}', re.S)
EXPR = re.compile(r'\{\{(-?)(.*?)(-?)\}\}', re.S)


def flatten_text(text):
    """Render Jinja text with every branch kept, the way Ansible renders it
    (trim_blocks), so {# #} comments, macros and whitespace control behave as
    on a host. Each {{ expr }} (except a call to a macro defined in the text)
    renders as a unique __Jn__ token; every if/elif/else branch renders; every
    for loop runs once. Returns (text, {token: expr, 'loop:x': iterable})."""
    from jinja2 import ChainableUndefined, Environment
    macros = set(re.findall(r'\{%-?\s*macro\s+(\w+)', text))
    exprs = {f'loop:{m.group(1)}': m.group(2)
             for m in re.finditer(r'\{%-?\s*for\s+(\w+)\s+in\s+([\w.]+)', text)}
    # A line holding only an expression is a block of rendered YAML (to_nice_yaml
    # and the like): it has no structure to check here, so it is dropped.
    text = re.sub(r'^[ \t]*\{\{(?!\s*(?:%s)\s*\()[^}]*\}\}[ \t]*\n' % '|'.join(map(re.escape, macros or {'\0'})),
                  '', text, flags=re.M)

    def expr(m):
        body = m.group(2).strip()
        call = re.match(r'(\w+)\s*\(', body)
        if call and call.group(1) in macros:
            return m.group(0)
        key = f'__J{len(exprs)}__'
        exprs[key] = body
        return '{{%s "%s" %s}}' % (m.group(1), key, m.group(3))
    text = EXPR.sub(expr, text)

    stack = []

    def tag(m):
        lead, name, rest, trail = m.groups()
        wrap = lambda body: '{%%%s %s %s%%}' % (lead, body, trail)
        if name in ('if', 'for', 'macro', 'filter', 'call'):
            stack.append(name)
        if name == 'if':
            return wrap('if true')
        if name in ('elif', 'else') and stack and stack[-1] == 'if':
            return wrap('endif') + wrap('if true')
        if name == 'else' and stack and stack[-1] == 'for':  # for-else: render the else body too
            return wrap('endfor') + wrap('for _p22f in [0]')
        if name == 'for':
            names = [n.strip() for n in rest.split(' in ', 1)[0].split(',')]
            item = '"__loop__"' if len(names) == 1 else '(%s)' % ', '.join(['"__loop__"'] * len(names))
            return wrap('for %s in [%s]' % (', '.join(names), item))
        if name in ('endif', 'endfor', 'endmacro', 'endfilter', 'endcall') and stack:
            stack.pop()
        if name in ('include', 'import', 'from', 'extends'):
            raise AssertionError(f'{{% {name} %}} is not handled by this guard')
        return m.group(0)
    text = TAG.sub(tag, text)

    class Stubbed(dict):
        """Jinja's own filters and tests, and a stub for any other (Ansible's):
        a filter returns its input, a test is true. Only {% set %} expressions
        and loop sources reach them; every {{ }} is a token already."""
        def __init__(self, real, stub):
            super().__init__(real)
            self.stub = stub

        def __contains__(self, key):
            return True

        def __getitem__(self, key):
            return dict.get(self, key, self.stub)

        def get(self, key, default=None):
            return dict.get(self, key, self.stub)
    env = Environment(trim_blocks=True, undefined=ChainableUndefined, keep_trailing_newline=True)
    env.filters = Stubbed(env.filters, lambda value, *a, **k: value)
    env.tests = Stubbed(env.tests, lambda *a, **k: True)
    env.globals.update(lookup=lambda *a, **k: '', query=lambda *a, **k: [])
    return env.from_string(text).render(), exprs


class MergingLoader(yaml.SafeLoader):
    """Every branch of an if/else renders, so one key can appear twice (two
    `middlewares:` blocks under `http:`). Plain YAML keeps only the last, which
    would hide whatever the first one defines; merge them instead."""


def _merge(a, b):
    if isinstance(a, dict) and isinstance(b, dict):
        out = dict(a)
        for k, v in b.items():
            out[k] = _merge(out[k], v) if k in out else v
        return out
    if isinstance(a, list) and isinstance(b, list):
        return a + [x for x in b if x not in a]
    return b


def _construct_merged_mapping(loader, node, deep=False):
    loader.flatten_mapping(node)
    out = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=True)
        value = loader.construct_object(value_node, deep=True)
        out[key] = _merge(out[key], value) if key in out else value
    return out


MergingLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _construct_merged_mapping)


def load_merged(text):
    return yaml.load(text, Loader=MergingLoader)


def flatten_template(path):
    """A Traefik template with every Jinja branch kept: (doc, exprs), see flatten_text."""
    text, exprs = flatten_text(path.read_text())
    return load_merged(text), exprs


def assigned_values(var):
    """The role default and every literal value ops/ansible assigns to var."""
    pattern = re.compile(r'^\s*' + re.escape(var) + r'\s*[:=]\s*["\']?([^"\'\s#]+)', re.M)
    values = set()
    for path in (ROOT / 'ops/ansible').rglob('*'):
        if path.is_file() and path.suffix in ('.yml', '.yaml', '.ini', ''):
            try:
                values.update(pattern.findall(path.read_text()))
            except (UnicodeDecodeError, OSError):
                continue
    return values


def assigned_lists(var):
    """Every value any YAML file under ops/ansible gives var (at any depth)."""
    found = []

    def walk(node):
        if isinstance(node, dict):
            for k, v in node.items():
                if k == var:
                    found.append(v)
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)
    for path in (ROOT / 'ops/ansible').rglob('*'):
        if path.is_file() and path.suffix in ('.yml', '.yaml') and var in path.read_text(errors='replace'):
            try:
                walk(yaml.safe_load(path.read_text()))
            except yaml.YAMLError:
                found.append(f'<unparsed {path.relative_to(ROOT)}>')
    return found


SUBNET_VAR = 'mgmt_docker_network_subnet'
# The only sources an IP allowlist may admit. 255.255.255.255/32 is the
# refuse-everything list (mgmt-read-refuse).
ALLOWED_SOURCES = {'100.64.0.0/10', '255.255.255.255/32', '{{ %s }}' % SUBNET_VAR}
# A LAN goatnet admits its own site LAN on purpose (lan_allowlist_cidr is mandatory).
LAN_EXCEPTION = ('templates/lan-traefik/lan-mgmt-dynamic.yml.j2', 'lan-mesh-allowlist', 'lan_allowlist_cidr')


def resolve_source(entry, exprs, depth=0):
    """The literal sources one sourceRange entry can render to."""
    entry = str(entry).strip()
    tokens = re.findall(r'__J\d+__', entry)
    if not tokens:
        return {entry}
    if entry != tokens[0] or depth > 3:
        return {entry}
    expr = exprs.get(entry, '')
    loop = exprs.get(f'loop:{expr}')
    if loop:  # {% for cidr in var %} ... "{{ cidr }}": every value var is ever given
        out = set()
        for value in assigned_lists(loop):
            for item in (value if isinstance(value, list) else [value]):
                out |= resolve_literal(str(item))
        return out
    return {'{{ %s }}' % expr}


def resolve_literal(text):
    m = re.fullmatch(r'\{\{\s*([^}]*?)\s*\}\}', text.strip())
    return {'{{ %s }}' % m.group(1)} if m else {text.strip()}


def ip_allowlists():
    """[(file, name, middleware, exprs)] for every ipAllowList/ipWhiteList anywhere."""
    out = []
    for path in ROUTER_TEMPLATES:
        doc, exprs = flatten_template(path)
        for proto in ('http', 'tcp'):
            for name, mw in (((doc or {}).get(proto) or {}).get('middlewares') or {}).items():
                for kind in ('ipAllowList', 'ipWhiteList'):
                    if isinstance(mw, dict) and kind in mw:
                        out.append((str(path.relative_to(ROOT / 'ops')), name, mw[kind] or {}, exprs))
    return out


def mesh_only_entrypoints():
    """Entrypoints that carry at least one router behind an IP allowlist."""
    allowlists, eps = allowlist_names(), set()
    for _, _, _, router, _ in all_routers():
        used = {str(m).split('@')[0] for m in (router or {}).get('middlewares') or []}
        if used & allowlists:
            eps |= {str(e) for e in (router or {}).get('entryPoints') or [] if '__J' not in str(e)}
    return eps


def all_routers():
    """[(file, proto, name, router, exprs)] over every Traefik router template."""
    out = []
    for path in ROUTER_TEMPLATES:
        doc, exprs = flatten_template(path)
        for proto in ('http', 'tcp', 'udp'):
            for name, router in (((doc or {}).get(proto) or {}).get('routers') or {}).items():
                out.append((str(path.relative_to(ROOT / 'ops')).removeprefix('ansible/'), proto, name, router, exprs))
    return out


def allowlist_names():
    """Every IP-allowlist middleware, and every chain that contains one."""
    names, chains = set(), {}
    for path in ROUTER_TEMPLATES:
        doc, _ = flatten_template(path)
        for proto in ('http', 'tcp'):
            for name, mw in (((doc or {}).get(proto) or {}).get('middlewares') or {}).items():
                if isinstance(mw, dict) and ('ipAllowList' in mw or 'ipWhiteList' in mw):
                    names.add(name)
                elif isinstance(mw, dict) and isinstance(mw.get('chain'), dict):
                    chains[name] = {str(m).split('@')[0] for m in mw['chain'].get('middlewares') or []}
    grew = True
    while grew:
        grew = False
        for name, members in chains.items():
            if name not in names and members & names:
                names.add(name)
                grew = True
    return names


class EnrollmentEntrypoint(unittest.TestCase):
    """Every relay-borne request arrives as 172.x.0.1 and passes the allowlist."""

    def test_no_comment_claims_the_allowlist_backstops_the_relay(self):
        text = DYNAMIC.read_text()
        for claim in ("isn't in 100.64.0.0/10", 'allowlist 403s the public-via-rathole'):
            self.assertFalse(claim in text, f"mgmt-dynamic.yml.j2 again claims: {claim}")

    def test_every_router_template_parses(self):
        names = {str(p.relative_to(ROOT / 'ops')).removeprefix('ansible/') for p in ROUTER_TEMPLATES}
        self.assertIn('templates/traefik-acme/mgmt-dynamic.yml.j2', names)
        for key in TEMPLATED_ENTRYPOINTS:
            self.assertIn(key[0], names)
        self.assertGreater(len(names), 12, 'found too few router templates; the guard is blind')
        for path in ROUTER_TEMPLATES:
            doc, _ = flatten_template(path)
            self.assertIsInstance(doc, dict, path)

    def test_every_router_is_pinned_and_none_on_enrollment_relies_on_an_allowlist(self):
        routers, allowlists = all_routers(), allowlist_names()
        self.assertGreater(sum(f.endswith('mgmt-dynamic.yml.j2') for f, *_ in routers), 20)
        self.assertIn('mesh-source-allowlist', allowlists)
        self.assertIn('icam-mtls-proxy-mesh-only', allowlists)
        self.assertIn('mesh-only-allowlist', allowlists)  # a chain around mesh-source-allowlist
        on_enrollment, seen_templated = 0, set()
        for path, proto, name, router, exprs in routers:
            where = f'{path} {proto} router {name}'
            eps = router.get('entryPoints') if isinstance(router, dict) else None
            self.assertTrue(eps, f'{where} omits entryPoints: Traefik binds it to all')
            # A list-valued expression ({{ eps }}) arrives as one string: refuse it.
            self.assertIsInstance(eps, list, f'{where}: entryPoints must be a literal list, got {eps!r}')
            choices = [[]]
            for ep in eps:
                ep = str(ep)
                if '__J' not in ep:
                    choices = [c + [ep] for c in choices]
                    continue
                var = TEMPLATED_ENTRYPOINTS.get((path, name))
                expr = exprs.get(ep.strip())
                self.assertTrue(var and expr and JINJA_VAR.match(expr) and JINJA_VAR.match(expr).group(1) == var,
                                f'{where}: templated entryPoints {expr!r} is not in TEMPLATED_ENTRYPOINTS')
                seen_templated.add((path, name))
                values = assigned_values(var)
                self.assertTrue(values, f'{where}: {var} has no default anywhere')
                choices = [c + [v] for c in choices for v in sorted(values)]
            used = [str(m) for m in router.get('middlewares') or []]
            for chosen in choices:
                if 'enrollment' not in chosen:
                    continue
                on_enrollment += 1
                self.assertFalse(any('__J' in m for m in used),
                                 f'{where} on enrollment has a templated middleware: {used}')
                hit = {m.split('@')[0] for m in used} & allowlists
                self.assertFalse(hit, f'{where} on enrollment relies on {hit}')
        self.assertEqual(seen_templated, set(TEMPLATED_ENTRYPOINTS), 'a TEMPLATED_ENTRYPOINTS entry is stale')
        self.assertGreater(on_enrollment, 0)


class EffectiveAllowlists(unittest.TestCase):
    """What every IP allowlist, wherever defined, can actually admit."""

    def test_every_allowlist_admits_only_the_mesh_and_the_mgmt_subnet(self):
        lists = ip_allowlists()
        names = {(f, n) for f, n, _, _ in lists}
        self.assertIn(('ansible/templates/traefik-acme/mgmt-dynamic.yml.j2', 'mesh-source-allowlist'), names)
        self.assertIn(('ansible/roles/icam-mtls-proxy/templates/traefik-passthrough.yml.j2',
                       'icam-mtls-proxy-mesh-only'), names)
        for path, name, mw, exprs in lists:
            where = f'{path} {name}'
            # ipStrategy hands the decision to X-Forwarded-For, which any client sets.
            self.assertNotIn('ipStrategy', mw, f'{where}: an allowlist must judge the TCP peer')
            sources = mw.get('sourceRange') or []
            self.assertIsInstance(sources, list, where)
            self.assertTrue(sources, f'{where}: empty sourceRange')
            for entry in sources:
                for source in resolve_source(entry, exprs):
                    if (path.removeprefix('ansible/'), name) == LAN_EXCEPTION[:2] and \
                            source.startswith('{{ %s' % LAN_EXCEPTION[2]):
                        continue
                    self.assertIn(source, ALLOWED_SOURCES, f'{where} admits {source}')

    def test_no_extra_cidrs_default(self):
        # Rendered straight into mesh-source-allowlist: any value in the tree is a widening.
        self.assertEqual(assigned_lists('mesh_only_allowlist_extra_cidrs'), [])
        self.assertEqual(assigned_values('mesh_only_allowlist_extra_cidrs'), set())

    def test_icam_allowlist_default_is_the_mesh_and_mgmt_subnet(self):
        self.assertEqual(assigned_lists('icam_mtls_proxy_allow_cidrs'),
                         [['100.64.0.0/10', '{{ %s }}' % SUBNET_VAR]])

    def test_mesh_only_chains_resolve_to_an_allowlist(self):
        self.assertIn('mesh-only-allowlist', allowlist_names())
        self.assertIn('lan-mesh-allowlist', allowlist_names())


def load_runner():
    import runpy
    import sys
    sys.path.insert(0, str(ROOT / 'ops/validation'))
    try:
        runner = runpy.run_path(str(ROOT / 'ops/validation/run-acceptance.py'))
    finally:
        sys.path.pop(0)
    return runner


FAKE_SS = r'''#!/usr/bin/env python3
"""ss, as far as the listener rows use it: -t -l -n -H, -4/-6, and `sport = :PORT`."""
import sys
args = sys.argv[1:]
flags = "".join(a[1:] for a in args if a.startswith("-") and not a.startswith("--"))
if "t" not in flags or "l" not in flags:
    sys.exit(0)
port = None
if "sport" in args:
    i = args.index("sport")
    if args[i + 1:i + 2] == ["="]:
        port = args[i + 2].lstrip(":")
for line in open(sys.argv[0] + ".listing").read().splitlines():
    local = line.split()[3]
    v6 = local.startswith("[") or local.startswith("*")
    if ("4" in flags and v6) or ("6" in flags and not v6):
        continue
    if port is not None and local.rsplit(":", 1)[1] != port:
        continue
    print(line)
'''


def fake_ss(directory, listing):
    ss = directory / 'ss'
    ss.write_text(FAKE_SS)
    ss.chmod(0o755)
    (directory / 'ss.listing').write_text(listing + '\n')
    return ss


class LiveRows(unittest.TestCase):
    """The acceptance rows that read the mgmt host's own :443 listeners."""
    TOKEN = 'ipv6-listener-on-443'

    def rows(self):
        rows = yaml.safe_load((ROOT / 'ops/validation/acceptance-tests.yaml').read_text())['tests']
        return {r['id']: r for r in rows}

    def test_refusal_row_is_paired_with_the_ipv4_listener_positive(self):
        rows = self.rows()
        neg, pos = rows['mgmt-websecure-has-no-ipv6-listener'], rows['mgmt-websecure-listens-on-ipv4']
        self.assertTrue(neg['must_refuse'])
        self.assertEqual(neg['paired_positive'], 'mgmt-websecure-listens-on-ipv4')
        self.assertFalse(pos.get('must_refuse', False))
        self.assertEqual(pos['params']['expect'], '0.0.0.0:443')
        self.assertEqual(neg['params']['host'], pos['params']['host'])
        # The refusal is graded on `expect`: it must be the token the command echoes.
        self.assertEqual(neg['params']['expect'], self.TOKEN)
        self.assertIn(f'echo {self.TOKEN};', neg['params']['cmd'])
        # requires names a ledger component; a wrong one SKIPs the row on every site.
        self.assertEqual(neg.get('requires'), ['traefik'])
        self.assertEqual(pos.get('requires'), ['traefik'])

    def test_rows_apply_on_known_variables(self):
        rows = self.rows()
        want = {'mgmt-websecure-listens-on-ipv4': {'goatlite': False},
                'mgmt-websecure-has-no-ipv6-listener': {'goatlite': False},
                'relay-refuses-mesh-only-hosts': 'relay_public_ips'}
        generator = (ROOT / 'ops/validation/generate-values.py').read_text()
        truthy = load_runner()['truthy_applies']
        for rid, expr in want.items():
            self.assertEqual(rows[rid].get('applies_when'), expr, rid)
            for var in (expr if isinstance(expr, dict) else [expr]):
                self.assertIn(f'values["{var}"] =', generator, f'{rid}: {var} is never set')
        self.assertTrue(truthy(want['mgmt-websecure-listens-on-ipv4'], {'goatlite': False}))
        self.assertTrue(truthy(want['relay-refuses-mesh-only-hosts'], {'relay_public_ips': ['192.0.2.1']}))
        # The relay list comes from DNS; it must be the looked-up list, never a constant.
        self.assertEqual(re.findall(r'values\["relay_public_ips"\] = (.+)', generator), ['relays'])

    def test_a_mgmt_site_is_not_a_goatlite(self):
        import contextlib
        import io
        import runpy
        import tempfile
        gen = runpy.run_path(str(ROOT / 'ops/validation/generate-values.py'))
        with tempfile.TemporaryDirectory() as d:
            Path(d, 'gv').mkdir()
            Path(d, 'inv.ini').write_text('[demo_hosts]\nmgmt-demo-01 ansible_host=100.127.0.7 router_role=A\n')
            for extra, want in (('', False), ('goatlite_host: "192.168.8.10"\n', True)):
                Path(d, 'gv', 'vars.yml').write_text('site_dns_zone: netbird.demo.net\nmesh_ip_a: "100.127.0.7"\n' + extra)
                with contextlib.redirect_stderr(io.StringIO()):
                    values = gen['generate']('demo', gv_dir=Path(d, 'gv'), inv_path=Path(d, 'inv.ini'))
                self.assertIs(values['goatlite'], want, extra or 'mgmt site')

    def test_relay_row_targets_hosts_the_templates_route(self):
        row = self.rows()['relay-refuses-mesh-only-hosts']
        self.assertEqual(row['params']['protected_targets'], [
            {'name': 'dashboard.{{ site_id }}.{{ public_mgmt_fqdn }}', 'path': '/'},
            {'name': 'auth.{{ site_id }}.{{ public_mgmt_fqdn }}', 'path': '/sessions/whoami'},
            {'name': 'oauth2.{{ site_id }}.{{ public_mgmt_fqdn }}', 'path': '/.well-known/openid-configuration'}])
        allowlists = allowlist_names()
        routed = set()
        for path, _, _, router, exprs in all_routers():
            if not path.endswith('mgmt-dynamic.yml.j2') or 'websecure' not in (router.get('entryPoints') or []):
                continue
            if not {str(m).split('@')[0] for m in router.get('middlewares') or []} & allowlists:
                continue
            rule = re.sub(r'__J\d+__', lambda m: '{{ %s }}' % exprs[m.group(0)], str(router.get('rule', '')))
            routed |= set(re.findall(r'Host\(`([^`]+)`\)', rule))
        for target in row['params']['protected_targets']:
            self.assertIn(target['name'], routed, f"{target['name']} is not a mesh-only websecure host")

    def graded(self, listing, row_id='mgmt-websecure-has-no-ipv6-listener'):
        """A listener row's verdict through cmd_assert (and the runner's inversion for
        the refusal), with a fake ss serving listing (None = no ss on PATH)."""
        import os
        import shutil
        import tempfile
        from unittest.mock import patch
        runner = load_runner()
        probes = runner['probes']
        row = self.rows()[row_id]
        params = {k: v for k, v in row['params'].items() if k != 'host'}
        with tempfile.TemporaryDirectory() as d:
            if listing is None:
                for tool in ('sh', 'grep'):
                    Path(d, tool).symlink_to(shutil.which(tool))
                path = d
            else:
                fake_ss(Path(d), listing)
                path = f'{d}:/usr/bin:/bin'
            with patch.dict(os.environ, {'PATH': path}):
                verdict = probes.cmd_assert(params)
        if row_id == 'mgmt-websecure-has-no-ipv6-listener':
            verdict = runner['invert_refusal'](verdict, 'cmd-assert')
        return verdict.status, probes

    def test_refusal_row_grades_listener_shapes(self):
        cases = (('LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*', 'PASS'),
                 ('LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\nLISTEN 0 4096 [::]:443 [::]:*', 'FAIL'),  # dual-stack
                 ('LISTEN 0 4096 *:443 *:*', 'FAIL'),
                 ('LISTEN 0 4096 [fd99::1]:443 [::]:*', 'FAIL'),
                 ('LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\nLISTEN 0 4096 [::]:4430 [::]:*', 'PASS'),
                 (None, 'UNKNOWN'))
        for listing, want in cases:
            status, probes = self.graded(listing)
            self.assertEqual(status, getattr(probes, want), listing)

    def test_positive_row_grades_listener_shapes(self):
        for listing, want in (('LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\nLISTEN 0 4096 [::]:443 [::]:*', 'PASS'),
                              ('LISTEN 0 4096 [::]:443 [::]:*', 'FAIL'),
                              ('LISTEN 0 4096 0.0.0.0:4430 0.0.0.0:*', 'FAIL')):
            status, probes = self.graded(listing, 'mgmt-websecure-listens-on-ipv4')
            self.assertEqual(status, getattr(probes, want), listing)

    def test_fake_ss_honours_family_and_port(self):
        import subprocess
        import tempfile
        listing = 'LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*\nLISTEN 0 4096 [::]:443 [::]:*\nLISTEN 0 4096 *:8443 *:*'
        with tempfile.TemporaryDirectory() as d:
            ss = str(fake_ss(Path(d), listing))
            run = lambda *a: subprocess.run([ss, *a], capture_output=True, text=True).stdout.splitlines()
            self.assertEqual(len(run('-Hltn', 'sport', '=', ':443')), 2)
            self.assertEqual(run('-Hltn4', 'sport', '=', ':443'), ['LISTEN 0 4096 0.0.0.0:443 0.0.0.0:*'])
            self.assertEqual(run('-Hltn', '-6', 'sport', '=', ':443'), ['LISTEN 0 4096 [::]:443 [::]:*'])
            self.assertEqual(run('-Hltn', 'sport', '=', ':4443'), [])
            self.assertEqual(run('-Hlun', 'sport', '=', ':443'), [])


class RelayTargets(unittest.TestCase):
    def test_every_rathole_client_targets_the_enrollment_listener(self):
        # all.yml need not set it (the template default applies); if it does, only to the enrollment listener.
        shared = yaml.safe_load((ROOT / 'ops/ansible/group_vars/all.yml').read_text()) or {}
        self.assertEqual(shared.get('rathole_client_netbird_mgmt_local_addr', ENROLLMENT), ENROLLMENT)
        template = (ROOT / 'ops/ansible/templates/client.toml.j2').read_text()
        self.assertIn(f"rathole_client_netbird_mgmt_local_addr | default('{ENROLLMENT}')", template)
        for name in ('ops/relay/client.toml', 'ops/relay/setup-client.sh'):
            self.assertEqual(toml_local_addr((ROOT / name).read_text()), ENROLLMENT, name)
        # Every service stanza a client config renders, not only the first.
        addrs = re.findall(r'^local_addr\s*=\s*"([^"]+)"', template, re.M)
        self.assertEqual(addrs, [f"{{{{ rathole_client_netbird_mgmt_local_addr | default('{ENROLLMENT}') }}}}"])
        for name in ('ops/relay/client.toml', 'ops/relay/setup-client.sh'):
            text = (ROOT / name).read_text()
            for stanza, addr in re.findall(r'\[client\.services\.([\w-]+)\][^\[]*?local_addr\s*=\s*"([^"]+)"', text):
                port = addr.rpartition(':')[2]
                self.assertFalse(port.isdigit() and int(port) == 443, f'{name} [{stanza}] -> {addr}')

    def test_no_inventory_overrides_the_relay_target_to_websecure(self):
        key = re.compile(r'rathole_client_netbird_mgmt_local_addr\s*[:=]\s*["\']?([^"\'\s#]+)')
        for path in (ROOT / 'ops/ansible').rglob('*'):
            if path.suffix not in ('.yml', '.yaml', '.j2', '.ini', '') or not path.is_file():
                continue
            try:
                text = path.read_text()
            except (UnicodeDecodeError, OSError):
                continue
            for value in key.findall(text):
                if '{{' in value:
                    continue
                port = value.rpartition(':')[2]
                self.assertFalse(port.isdigit() and int(port) == 443, f'{path.relative_to(ROOT)}: {value}')


if __name__ == '__main__':
    unittest.main()
