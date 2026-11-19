import hashlib, sys, jinja2, json, re
class Any(jinja2.ChainableUndefined):
    def __int__(self): return 0
    def __float__(self): return 0.0
    def __iter__(self): return iter([])
    def __len__(self): return 0
    def __str__(self): return ''
    def __html__(self): return ''
def h(s, algo='sha1'): return hashlib.new(algo, str(s).encode()).hexdigest()
def to_bool(v,*a):
    if isinstance(v,bool): return v
    return str(v).strip().lower() in ('true','1','yes','on')
FILTERS = dict(hash=h, bool=to_bool, ternary=lambda v,a,b,*_: a if v else b,
               regex_replace=lambda s,p,r='',*a: re.sub(p,r,str(s)),
               to_json=lambda v,*a: json.dumps(v), to_nice_json=lambda v,*a: json.dumps(v,indent=2),
               mandatory=lambda v,*a: v)
def render(path, extra):
    env = jinja2.Environment(undefined=Any, keep_trailing_newline=True, trim_blocks=True, lstrip_blocks=True)
    env.filters.update(FILTERS)
    base = dict(zenoh_identity_model='per-uuid-slot-pool', zenoh_slot_pool_size=4,
                zenoh_slot_pool_salt='testsalt', zenoh_partner_read_partition=False,
                zenoh_partner_read_lanes={}, zenoh_universal_read_cns=[])
    base.update(extra)
    with open(path) as source_file:
        return env.from_string(source_file.read()).render(**base)
mode, mine, main = sys.argv[1], sys.argv[2], sys.argv[3]
if mode=='off-diff':
    # GHSA-rr3p changes the default slot-pool render on purpose (slot subjects
    # from the registry fragment, an enumerated trial-peer-read, one slot-read
    # policy). The guard is that NOTHING ELSE differs from main, and that the
    # rr3p side carries those changes correctly (slot-fragment/rr3p_render_diff.py).
    import os
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'slot-fragment'))
    import rr3p_render_diff
    a=render(main,{}); b=render(mine,{})
    same = rr3p_render_diff.equivalent(a, b)
    problems = rr3p_render_diff.intended(b)
    print('OFF identical apart from the rr3p-intended differences:', same and not problems)
    if not same:
        for l in rr3p_render_diff.first_difference(a, b): print(l)
    for p in problems: print('rr3p render problem:', p)
elif mode=='on':
    anchor0=h('testsalt:0','sha256')[:32]
    b=render(mine,{'zenoh_partner_read_partition':True,'zenoh_partner_read_lanes':{anchor0:['ZAGROS']},'zenoh_universal_read_cns':['playground-peer']})
    with open('/tmp/on_render.json5','w') as source_file:
        source_file.write(b)
    print('anchor0:',anchor0)
    print('partner policy on its slot subject:', ('"partner-deliver-%s", "audit-operator-read-deny"],\n        subjects: ["slot-%s"]'%(anchor0,anchor0)) in b)
    print('partner-r rule :', ('id: "partner-r-%s"'%anchor0) in b)
    print('partner-deliver:', ('id: "partner-deliver-%s"'%anchor0) in b)
    print('release/ZAGROS/ :', 'release/ZAGROS/**' in b)
    tpr=b.split('id: "trial-peer-read"')[1].split('],')[0]
    print('trial-peer-read narrowed (cert_common_names, no link_protocols):', 'cert_common_names' in tpr and 'link_protocols' not in tpr)
    print('partner EXCLUDED from universal read list:', anchor0 not in tpr)
    print('playground-peer IN universal read list   :', 'playground-peer' in tpr)
    # partner-r must have NO put/delete (no write grant)
    prr=b.split('id: "partner-r-%s"'%anchor0)[1].split('},')[0]
    print('partner-r has NO put/delete (read-only):', '"put"' not in prr and '"delete"' not in prr)
