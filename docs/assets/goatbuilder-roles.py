#!/usr/bin/env python3
"""Draw goatbuilder-roles-light.svg and goatbuilder-roles-dark.svg: the machine
roles of a goatnet, solid where goatbuilder builds one today and dashed where it
is designed as a target type. Change a role's status here (the sixth field of
its row: 1 built, 0 designed) and run:

    python3 -I docs/assets/goatbuilder-roles.py            # write both files
    python3 -I docs/assets/goatbuilder-roles.py --check    # fail if either differs
"""
import math
import pathlib
import re
import sys

# key, label lines, group, shape, families, built, does, today, link
T=[
('core',['core'],'Host site','host','host-class, cloud',1,'Management plane: mesh control, identity, the fabric router. Runs as an A and B pair.','goatbuilder standup builds the first one; goatbuilder join-node --class core adds one.',None),
('obs',['obs'],'Host site','host','host-class, cloud',1,'Observability and storage for the site. Runs as an A and B pair.','goatbuilder standup; goatbuilder join-node --class obs.',None),
('phone',['phone'],'Host site','phone','phone',1,'A phone as a full core joiner, reached over a USB cable until it is on the mesh.','goatbuilder join-node --class phone --via usb.',None),
('own',['your own','role'],'Your own','either','any family you declare',0,'A role or service that stays private. Its file lives in your overlay and its plays in your own repository; goatbuilder fetches them with your access and checks the digest and signer.','Not built.',None),
('breakglass',['break-glass'],'Access','small','small computer',1,"The way in to the site's goat machines when the mesh and the control plane are both down.",'goatbuilder standup phase 14; goatbuilder join-node --class breakglass.',None),
('network-jump',['jump box'],'Access','small','small computer',1,'The way in to switches, routers and out-of-band management. Network gear only.','goatbuilder join-node --class network-jump.',None),
('processors',['goat-','processors'],'Processing','host','host-class',0,'Correlation, fusion, filtering and quality on translated track data.','Its own repository runs the engines.','goat-processors'),
('federation',['federation','boundary'],'Boundary','either','small computer or host-class',0,'The edge router that peers with another goatnet.','goatbuilder federate sets up the peering on a host that already exists.',None),
('simple',['simple','boundary'],'Boundary','either','small computer or host-class',0,'Sends selected data out to a receiver that does not run goat.','An Ansible role, simple-bn.yml, run by hand.',None),
('octo-pod',['octo-pod'],'Edge','either','small computer or host-class',0,'Edge gateway: takes in feeds, records and audits them, and releases chosen feeds to the fabric.','Its own repository installs it.','octo-pod'),
('sensor',['Billie node'],'Edge','kids','small computer',0,'The sensor node. It runs the Open Billie platform and one billie kid per sensor: a kid decodes that sensor and hands over a track, and the platform writes CoT, ASTERIX CAT062 or SAPIENT to the fabric.','Open Billie installs it: billie kid install, configure, activate.','open-billie'),
('scapegoat',['scapegoat'],'Edge','small','small computer',0,'A service on a Billie node, for a sensor that sends nothing until a C2 system speaks to it first. scapegoat speaks first. Protocols are plugins, and a private one stays in its own overlay.','Its own installer. The public repository is not published yet.',None),
('vendor',['vendor','gateway'],'Edge','small','small computer',0,'Runs the vendor bridges, one package per vendor.','Its own repository holds the bridges.','open-entity-gateway'),
('first-bite',['first-bite'],'Edge','either','small computer or host-class',0,'Decodes binary messages from a network tap and hands them to authorized software.','Its own repository installs it.','first-bite'),
('tap',['network','tap'],'Edge','small','small computer or host-class',0,'Records one Ethernet link, and can hand chosen traffic to a decoder. The machine must declare its mode: listen fails open, inline does not.','first-bite holds the capture service; the port setup is not in a public repository.','first-bite'),
]
C30=math.cos(math.pi/6); U=35; OX=430; OY=62
def P(x,y,z=0): return ((x-y)*C30*U+OX, (x+y)*.5*U-z*U+OY)
def pts(l): return ' '.join('%.1f,%.1f'%p for p in l)
def poly(l,cls): return f'<polygon class="{cls}" points="{pts(l)}"/>'
def box(x,y,w,d,h,z=0):
    a=[P(x,y+d,z),P(x+w,y+d,z),P(x+w,y+d,z+h),P(x,y+d,z+h)]
    b=[P(x+w,y,z),P(x+w,y+d,z),P(x+w,y+d,z+h),P(x+w,y,z+h)]
    t=[P(x,y,z+h),P(x+w,y,z+h),P(x+w,y+d,z+h),P(x,y+d,z+h)]
    return poly(a,'sl')+poly(b,'sr')+poly(t,'st')
SP=3.2; GAP=.75
def rowy(r): return r*(SP+GAP)
cells=[]
for i,t in enumerate(T):
    if i<3: r,c=0,i
    elif i==3: r,c=0,4
    elif i<9: r,c=1,[0,1,2,3.25,4.5][i-4]
    else: r,c=2,i-9
    cells.append((c*SP,rowy(r)))
def pad(x,y,w,label):
    d=SP
    s=poly([P(x,y+d),P(x+w,y+d),P(x+w,y+d,-.16),P(x,y+d,-.16)],'pe')+poly([P(x+w,y),P(x+w,y+d),P(x+w,y+d,-.16),P(x+w,y,-.16)],'pe')
    s+=poly([P(x,y),P(x+w,y),P(x+w,y+d),P(x,y+d)],'pd')
    return s
def machine(i):
    k,lab,grp,sh,fam,built,*_=T[i]; x,y=cells[i]
    if sh=='host':
        w=d=1.25;h=1.55
    elif sh=='small': w=d=.95;h=.5
    elif sh=='kids': w=d=1.2;h=.5
    elif sh=='either': w=d=1.1;h=.95
    else: w=.75;d=.16;h=1.3
    bx=x+(SP-w)/2-.35; by=y+(SP-d)/2-.35
    s=box(bx,by,w,d,h)
    if sh=='kids':
        for ax,ay in ((.08,.08),(.62,.08),(.08,.62),(.62,.62)): s+='<g class="kid">'+box(bx+ax,by+ay,.42,.42,.36,h)+'</g>'
    if sh=='host':
        for zz in (.35,.7,1.05):
            a=P(bx+.12,by+d,zz);b=P(bx+w-.12,by+d,zz); s+=f'<path class="dt" d="M{a[0]:.1f} {a[1]:.1f}L{b[0]:.1f} {b[1]:.1f}"/>'
        a=P(bx+w-.28,by+d,1.3); s+=f'<circle class="led" cx="{a[0]:.1f}" cy="{a[1]:.1f}" r="3"/>'
    elif sh in('small','either','kids'):
        a=P(bx+.22,by+d,h*.5); s+=f'<circle class="led" cx="{a[0]:.1f}" cy="{a[1]:.1f}" r="3"/>'
        for q in (.5,.72):
            a=P(bx+q,by+d,h*.5);b=P(bx+q+.12,by+d,h*.5); s+=f'<path class="dt" d="M{a[0]:.1f} {a[1]:.1f}L{b[0]:.1f} {b[1]:.1f}"/>'
        if sh=='either':
            a=P(bx+w,by+.2,h*.6);b=P(bx+w,by+d-.2,h*.6); s+=f'<path class="dt" d="M{a[0]:.1f} {a[1]:.1f}L{b[0]:.1f} {b[1]:.1f}"/>'
    else:
        a=P(bx+.12,by+d,.2);b=P(bx+w-.12,by+d,.2);c=P(bx+w-.12,by+d,h-.15);e=P(bx+.12,by+d,h-.15); s+=poly([a,b,c,e],'scr')
    foot=P(x+SP/2+.35,y+SP/2+.35); cy=foot[1]+14
    t=''
    for j,l in enumerate(lab): t+=f'<text class="ml" x="{foot[0]:.1f}" y="{cy+j*19:.1f}" text-anchor="middle">{l}</text>'
    ring=[P(bx-.28,by-.28),P(bx+w+.28,by-.28),P(bx+w+.28,by+d+.28),P(bx-.28,by+d+.28)]
    cls='built' if built else 'plan'
    return f'<g class="mach {cls}" data-k="{k}" id="m-{k}">'+poly([P(x+.1,y+.1),P(x+SP-.1,y+.1),P(x+SP-.1,y+SP-.1),P(x+.1,y+SP-.1)],'hit')+poly(ring,'ring')+f'<g class="body">{s}</g>{t}</g>'
def center(i,off=0):
    x,y=cells[i]; return P(x+SP/2-.35,y+SP/2-.35)
def scene(inner_class='scene', extra=''):
    W,H=1080,612
    s=f'<svg class="{inner_class}" viewBox="30 0 {W} {H}" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Thirteen machine roles on one site plan, and a place for your own. Solid, built by goatbuilder today: core, obs, phone, break-glass, network-gear jump box. Dashed, designed as goatbuilder target types: goat-processors, federation boundary, simple boundary, octo-pod, Billie node with its kids, scapegoat, vendor gateway, first-bite, network tap, and a role of your own.">{extra}'
    for lab,x,y,w in [('HOST SITE',0,0,3),('YOUR OWN',3,0,3),('ACCESS',0,1,2),('PROCESSING',2,1,1),('BOUNDARY',3,1,3),('EDGE',0,2,6)]:
        px=x*SP+.06; pw=w*SP-.12
        s+=pad(px,rowy(y),pw,lab)
    # links through the gutters, drawn behind machines
    for y in range(2):
        yy=rowy(y)+SP+GAP/2; a=P(.06,yy);b=P(6*SP-.06,yy); s+=f'<path class="mesh" d="M{a[0]:.1f} {a[1]:.1f}L{b[0]:.1f} {b[1]:.1f}"/>'
    # group names along the left edges of the rows and the right edge for the second group
    for lab,x,y in [('HOST SITE',0,0),('ACCESS',0,1),('EDGE',0,2)]:
        p=P(0,rowy(y)+SP*.5); s+=f'<text class="gl" x="{p[0]-20:.1f}" y="{p[1]-18:.1f}" text-anchor="end">{lab}</text>'
    for lab,y in [('YOUR OWN',0),('BOUNDARY',1)]:
        p=P(6*SP,rowy(y)+SP*.5); s+=f'<text class="gl" x="{p[0]+16:.1f}" y="{p[1]+14:.1f}" text-anchor="{"start" if y else "start"}">{lab}</text>'
    for i in range(len(T)): s+=machine(i)
    return s+'</svg>'
SVGCSS='''
.scene{display:block;width:100%;height:auto;font-family:var(--font);overflow:visible}
.scene .pd{fill:var(--panel);stroke:var(--rule);stroke-width:1.2}.scene .pe{fill:var(--rule)}
.scene .mesh{stroke:var(--accent);stroke-width:2.2;fill:none;stroke-dasharray:3 7;stroke-linecap:round}
.scene .gl{font:700 14px var(--mono);letter-spacing:.08em;fill:var(--quiet)}
.scene .ml{font-size:17px;font-weight:700;fill:var(--ink);paint-order:stroke fill;stroke:var(--panel);stroke-width:4px;stroke-linejoin:round}
.scene .hit{fill:transparent}.scene .ring{fill:none;stroke:none}
.scene .body polygon{stroke-linejoin:round}
.scene .built .st{fill:var(--accent-soft);stroke:var(--accent);stroke-width:1.8}
.scene .built .sl{fill:var(--accent);stroke:var(--accent);stroke-width:1.8}
.scene .built .sr{fill:var(--accent);fill-opacity:.78;stroke:var(--accent);stroke-width:1.8}
.scene .built .scr{fill:var(--accent-soft)}
.scene .built .dt{stroke:var(--on-accent);stroke-width:2;stroke-linecap:round;opacity:.75}
.scene .built .led{fill:var(--on-accent)}
.scene .plan .st,.scene .plan .sl,.scene .plan .sr{fill:var(--paper);stroke:var(--ink);stroke-width:1.5;stroke-dasharray:5 4}
.scene .plan .kid polygon{stroke-dasharray:none;stroke-width:1.2}
.scene .plan .dt{stroke:var(--line);stroke-width:1.5}.scene .plan .led{fill:none;stroke:var(--line);stroke-width:1.5}
'''

LIGHT = {'ink': '#1b2118', 'quiet': '#525a4a', 'rule': '#d3d6c4', 'line': '#7f8770', 'paper': '#f1f2e8', 'panel': '#e6e8d9', 'accent': '#1c6a3d', 'accent-soft': '#d9e8d3', 'on-accent': '#ffffff', 'font': '"Helvetica Neue", Helvetica, Arial, "Liberation Sans", sans-serif', 'mono': 'ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'}
DARK = {'ink': '#e6e9da', 'quiet': '#a7ae98', 'rule': '#2f3728', 'line': '#7d866e', 'paper': '#12160f', 'panel': '#1a2016', 'accent': '#74cf93', 'accent-soft': '#1a3223', 'on-accent': '#0e140c', 'font': '"Helvetica Neue", Helvetica, Arial, "Liberation Sans", sans-serif', 'mono': 'ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'}


def drawing(t):
    st = re.sub(r'var\(--([a-z-]+)\)', lambda m: t[m[1]].replace('"', "'"), SVGCSS.replace('.scene{display:block;width:100%;height:auto;', '.scene{'))
    leg = f'<g transform="translate(60 540)"><rect width="18" height="18" rx="3" fill="{t["accent"]}"/><text x="28" y="15" font-size="17" fill="{t["ink"]}">goatbuilder builds it today</text><rect y="28" width="18" height="18" rx="3" fill="{t["paper"]}" stroke="{t["ink"]}" stroke-width="1.6" stroke-dasharray="4 3"/><text x="28" y="43" font-size="17" fill="{t["ink"]}">designed as a target type</text></g>'
    return scene('scene', f'<style>{st}</style><rect x="30" width="1080" height="612" rx="12" fill="{t["paper"]}"/>').replace('</svg>', leg + '</svg>') + '\n'


if __name__ == '__main__':
    here, bad = pathlib.Path(__file__).resolve().parent, 0
    for name, t in (('light', LIGHT), ('dark', DARK)):
        p = here / f'goatbuilder-roles-{name}.svg'
        if '--check' in sys.argv:
            bad |= not p.exists() or p.read_text() != drawing(t)
        else:
            p.write_text(drawing(t))
    sys.exit('goatbuilder-roles: a figure differs from its script' if bad else 0)
