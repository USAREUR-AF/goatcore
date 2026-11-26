#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Boundary monitor: a small read-only page over the status files a boundary node writes.

Reads every *.json in --status-dir (schema goat.boundary-status/v1), one per boundary
service on this node, and serves:
  GET /                      the page (refreshes itself every 5 s)
  GET /api/v1/status         the status documents as JSON (/api/status is the same)
  GET /api/v1/feeds/<feed>   one feed
  GET /api/v1/feedback       what the receiver has told us (goat.bn-feedback/v1)
  GET /api/v1/health         200 when every feed is sending (a relay with no receiver: holding), 503 with the problems
  GET /api/v1/modifications  per feed: what arrives, what the receiver gets, and what this node changed
  GET /api/v1/sim-status     per feed: real or simulated as declared out of band, and whether the SIM bit agrees
  GET /api/v1/openapi.json   the OpenAPI 3 description of the five above
  GET /api/docs              Swagger UI over it, with "Try it out" on
  GET /healthz               ok

Bind it to the node's mesh address only; it never binds 0.0.0.0 unless told to.
Python standard library only.
"""
import argparse
import glob
import json
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PAGE = r"""<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Boundary monitor</title>
<style>
:root{--bg:#f6f7f9;--card:#fff;--ink:#16202a;--mute:#5b6875;--line:#dde2e8;
--ok:#1f8a4c;--warn:#b7791f;--bad:#c53030;--f0:#2b6cb0;--f1:#c05621;--f2:#6b46c1;--f3:#2c7a7b}
@media (prefers-color-scheme:dark){:root{--bg:#0f1418;--card:#172028;--ink:#e6edf3;--mute:#93a1ad;--line:#2a3640;
--f0:#63b3ed;--f1:#f6ad55;--f2:#b794f4;--f3:#4fd1c5}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 system-ui,-apple-system,Segoe UI,sans-serif}
main{max-width:1100px;margin:0 auto;padding:20px 16px}
h1{font-size:20px;margin:0 0 4px}.sub{color:var(--mute);margin-bottom:16px}
.grid{display:grid;gap:12px;grid-template-columns:repeat(auto-fill,minmax(300px,1fr))}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:14px;border-top:4px solid var(--c)}
.hd{display:flex;justify-content:space-between;align-items:baseline;gap:8px}
.name{font-weight:650;font-size:15px}.ids{color:var(--mute);font-size:12px}
.badge{font-size:12px;padding:2px 8px;border-radius:99px;color:#fff;white-space:nowrap}
.s-sending,.s-holding{background:var(--ok)}.s-stalled,.s-starting,.s-reconnecting,.s-waiting-for-input{background:var(--warn)}.s-receiver-refusing,.s-send-error,.s-stopped{background:var(--bad)}
table{width:100%;border-collapse:collapse;margin-top:8px;font-variant-numeric:tabular-nums}
th,td{text-align:right;padding:3px 4px;border-bottom:1px solid var(--line)}th:first-child,td:first-child{text-align:left;color:var(--mute);font-weight:500}
th{color:var(--mute);font-weight:500;font-size:12px}
.kinds{margin-top:8px;display:flex;flex-wrap:wrap;gap:6px}.k{font-size:12px;border:1px solid var(--line);border-radius:6px;padding:1px 6px}
.ack{margin-top:8px;font-size:12px;color:var(--mute)}svg{display:block;margin-top:8px;width:100%;height:42px}
.node{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px;margin-bottom:12px;display:flex;flex-wrap:wrap;gap:18px}
.node b{display:block;font-size:12px;color:var(--mute);font-weight:500}
.err{color:var(--bad)}
.ident{display:grid;grid-template-columns:auto 1fr;gap:2px 10px;font-size:12.5px;margin:6px 0 8px;padding:6px 8px;border-radius:6px;border:1px solid var(--line)}
.ident>div{display:contents}.ident b{color:var(--mute);font-weight:600}
.ident .cls{font-weight:700;letter-spacing:.06em}
.ident-real .cls{color:#1f6b34}.ident-synthetic .cls{color:#5b6875}
.chk-mismatch{border-color:#8a5a00;background:rgba(176,122,0,.08)}
</style></head><body><main>
<h1>Boundary monitor</h1><div class="sub" id="sub">loading…</div><div class="sub"><a href="/api/docs" style="color:inherit">API and examples</a></div>
<div id="nodes"></div>
<script>
const W=["1m","15m","1h","24h"];
const fmt=n=>n==null?"–":n>=1e6?(n/1e6).toFixed(1)+"M":n>=1e4?(n/1e3).toFixed(1)+"k":esc(n);
// Every string from a status file or from the receiver goes through esc() before it reaches innerHTML.
const esc=x=>String(x??"").replace(/[&<>"']/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const ago=t=>t==null?"never":esc(Math.max(0,Math.round(Date.now()/1000-t)))+" s ago";
const dur=s=>{const d=Math.floor(s/86400),h=Math.floor(s%86400/3600),m=Math.floor(s%3600/60);return esc((d?d+"d ":"")+h+"h "+m+"m")};
function spark(pm,col){if(!pm.length)return"";const v=pm.map(x=>x[1]),mx=Math.max(1,...v),w=300,h=40,bw=w/v.length;
 const bars=v.map((y,i)=>{const bh=y?Math.max(1.5,(y/mx)*(h-4)):0;return `<rect x="${(i*bw+0.5).toFixed(1)}" y="${(h-bh).toFixed(1)}" width="${(bw-1).toFixed(1)}" height="${bh.toFixed(1)}" fill="${col}"/>`}).join("");
 return `<svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="datagrams per minute, last 60 minutes"><title>datagrams per minute, last 60 minutes (peak ${esc(mx)})</title><line x1="0" y1="${h-0.5}" x2="${w}" y2="${h-0.5}" stroke="var(--line)"/>${bars}</svg><div class="ack" style="margin-top:2px">datagrams per minute · last 60 min · peak ${esc(mx)}</div>`}
function fbpanel(fb){if(!fb)return"";const h=fb.hello&&fb.hello.msg,st=fb.status&&fb.status.msg,rq=fb.request&&fb.request.msg;
 const kv=o=>o?Object.entries(o).filter(([k])=>k!=="type").map(([k,v])=>`${esc(k)}: ${esc(Array.isArray(v)?v.join(", "):v)}`).join(" · "):"–";
 return `<div class="node"><div><b>Receiver feedback</b>${fb.last_heard?"last heard "+ago(fb.last_heard):"nothing received yet"}${fb.port?" · port "+esc(fb.port):""} · ${esc(fb.counts.accepted)} accepted, ${esc(fb.counts.rejected)} rejected</div>
 <div><b>Hello</b>${kv(h)}</div><div><b>Status</b>${kv(st)}</div><div><b>Request</b>${kv(rq)}</div></div>`}
function idpanel(f){const id=f.identity;if(!id)return"";
 const cls=id.data_class==="real"?"REAL":"SIMULATED";
 const exp=`SAC ${esc(id.declared.sac??'any')} · SIC ${esc(id.declared.sic??'any')}`;
 const seen=id.observed.length?id.observed.map(o=>`SAC ${esc(o.sac??'none')} / SIC ${esc(o.sic??'none')} (${o.categories.map(c=>'CAT'+esc(String(c).padStart(3,'0'))).join(', ')}, ${fmt(o.count)} blocks, ${ago(o.last)})`).join("<br>"):"nothing seen yet";
 const chk=id.check==="ok"?"matches":id.check==="unseen"?"waiting for data":"MISMATCH";
 const ss=f.sim_status;const simrow=ss?`<div><b>Sim / real</b>${esc(String(ss.declared??"").toUpperCase())} · ${ss.basis==="operator"?"declared by "+esc(ss.set_by||"?")+" "+ago(ss.set_at)+(ss.note?" · "+esc(ss.note):""):"from the profile"}${ss.source_can_simulate&&!ss.source_marks_simulation?" · the source can run a simulation without marking it":""}${ss.agrees===false?'<br><span class="err">SIM bit '+(ss.sim_bit_sent?"set":"clear")+" does not match the declaration</span>":""}</div>`:"";
 return `<div class="ident ident-${esc(id.data_class)} chk-${esc(id.check)}"><div><b>Data</b><span class="cls">${cls}</span></div>
 <div><b>Declared</b>${exp}</div><div><b>Seen</b>${seen}</div><div><b>Check</b>${chk}${id.problems.length?'<br><span class="err">'+id.problems.map(esc).join('<br>')+'</span>':''}</div>${simrow}</div>`}
async function tick(){try{const r=await fetch("api/status",{cache:"no-store"});const docs=await r.json();
 document.getElementById("sub").textContent="Updated "+new Date().toLocaleTimeString()+" · "+docs.length+" boundary service(s) on this node";
 let html="";docs.forEach(d=>{
  html+=`<div class="node"><div><b>Node</b>${esc(d.node)} · ${esc(d.kind)}</div><div><b>Service</b>${esc(d.service)}</div><div><b>Uptime</b>${dur(d.uptime_s)}</div>
  <div><b>Receiver</b>${esc(d.receiver.label)}</div><div><b>Transport</b>${esc(d.receiver.transport)} · ${esc(d.receiver.format)}</div><div><b>Encryption</b>${esc(d.receiver.encryption)}</div>
  ${d.stale?'<div class="err"><b>Status file</b>stale: service not writing</div>':""}</div>${fbpanel(d.feedback)}<div class="grid">`;
  d.feeds.forEach((f,i)=>{const col=`var(--f${i%4})`;
   const rows=f.relay?[["From the sender","received"],["Wrong sender","rejected_source"],["Malformed","rejected_format"],["Held (no receiver)","held"],["Forwarded","datagrams"],["Bytes","bytes"],["Send errors","send_errors"],["Refused","refused"]]
    :[["Datagrams","datagrams"],["Detections","detections"],["Tracks","tracks"],["Service msgs","cat034"],["Bytes","bytes"],["Send errors","send_errors"],["Refused","refused"],["Dropped","dropped"]];
   html+=`<div class="card" style="--c:${col}"><div class="hd"><div><div class="name">${esc(f.title)}</div>
   <div class="ids">SAC ${esc(f.sac??'any')} · SIC ${esc(f.sic??'any')} · src port ${esc(f.src_port)} · ${f.relay?'relayed from the goatnet':'every '+esc(f.update_period_s)+'s'}${f.transport?' · '+esc(f.transport):''}</div></div>
   <span class="badge s-${esc(f.state)}">${esc(f.state)}</span></div>
   ${idpanel(f)}
   <table><tr><th></th>${W.map(w=>`<th>${w}</th>`).join("")}<th>total</th></tr>
   ${rows.map(([l,k])=>`<tr><td>${l}</td>${W.map(w=>`<td>${fmt((f.windows[w]||{})[k]||0)}</td>`).join("")}<td>${fmt(f.totals[k]||0)}</td></tr>`).join("")}</table>
   ${spark(f.per_minute,col)}
   <div class="kinds">${f.relay?`<span class="k">${f.relay.forwarding?'forwarding to the receiver':'holding: no receiver declared'}</span>`:Object.entries(f.live_targets_by_kind).map(([k,n])=>`<span class="k">${esc(k)}: ${esc(n)}</span>`).join("")||'<span class="k">no live targets</span>'}</div>
   <div class="ack">Last send ${ago(f.last_send)} · Ack: ${f.ack.mode==="app"?`receiver got ${fmt(f.ack.receiver_received)} of ${fmt(f.ack.we_sent_in_window)} in ${esc(f.ack.window_s)}s (${ago(f.ack.at)})`:esc(f.ack.note)}${f.ack.last_refused?" · last refused "+ago(f.ack.last_refused):""}
   ${f.last_error?`<br><span class="err">Last error: ${esc(f.last_error)}</span>`:""}</div></div>`});
  html+="</div>"});
 document.getElementById("nodes").innerHTML=html||"<p>No boundary service status found.</p>";
}catch(e){document.getElementById("sub").textContent="Monitor API unreachable: "+e}}
tick();setInterval(tick,5000);
</script></main></body></html>"""


# ---- OpenAPI description of the read-only API, and a Swagger UI page over it ----------
_FEED_EXAMPLE = {
    "feed": "tpq53", "title": "TPQ-53 (notional)", "sac": 0, "sic": 53, "src_port": 40053,
    "update_period_s": 1.0, "state": "sending", "last_send": 1795124016.7, "last_error": None,
    "transport": "udp", "connected": True,
    "ack": {"mode": "none", "note": "UDP one-way; receiver sends no acknowledgement", "last_refused": None},
    "live_targets_by_kind": {"projectile": 1},
    "totals": {"datagrams": 24473, "bytes": 646140, "cat034": 14448, "detections": 20112, "tracks": 12819,
               "send_errors": 0, "refused": 0, "dropped": 0},
    "windows": {"1m": {"datagrams": 174, "bytes": 6260, "cat034": 97, "tracks": 160, "detections": 207},
                "1h": {"datagrams": 6146, "bytes": 164509, "cat034": 3637, "tracks": 3293, "detections": 5141}},
    "per_minute": [[29918731, 102], [29918732, 112]],
    "sim_status": {"declared": "simulated", "basis": "profile", "set_at": None, "set_by": None, "note": None,
                   "source_can_simulate": True, "source_marks_simulation": False, "sim_bit_sent": True,
                   "agrees": True},
}
_SIM_EXAMPLE = [{"node": "boundary-host-a", "feed": "drone-a", "title": "Friendly drone", "sac": 7, "sic": 5,
                 "declared": "simulated", "basis": "operator", "set_at": 1795123000.0, "set_by": "ops desk",
                 "note": "the source's operators report a simulation run", "source_can_simulate": True,
                 "source_marks_simulation": False, "sim_bit_sent": False, "agrees": False}]
_FEEDBACK_EXAMPLE = {"schema": "goat.bn-feedback/v1", "last_heard": 1795124010.2,
                     "counts": {"accepted": 3, "rejected": 0},
                     "hello": {"system": "partner decoder", "contact": "ops desk"},
                     "status": {"receiving": True, "decoding": True, "positions_set": True},
                     "request": None, "acks": {"53": {"received": 58, "window_s": 60, "we_sent_in_window": 60}},
                     "port": 40099}
_STATUS_EXAMPLE = [{"schema": "goat.boundary-status/v1", "kind": "simple-bn", "node": "boundary-host-a",
                    "service": "simple-bn-sender",
                    "receiver": {"label": "partner-dev", "transport": "udp", "format": "ASTERIX CAT034/CAT048",
                                 "encryption": "none"},
                    "started": 1795109569.7, "now": 1795124017.6, "uptime_s": 14448, "stale": False,
                    "feeds": [_FEED_EXAMPLE], "feedback": _FEEDBACK_EXAMPLE}]
_J = "application/json"
_MOD_EXAMPLE = [{"node": "boundary-host-a", "feed": "radar-a", "title": "Radar A (real)", "state": "sending",
                 "source": "relayed",
                 "input": [{"sac": 9, "sic": 17, "categories": [62], "count": 278, "last": 1795713922.4}],
                 "output": [{"category": 48, "sac": 60, "sic": 7, "count": 22, "last": 1795713925.2},
                            {"category": 34, "sac": 60, "sic": 7, "count": 19, "last": 1795713924.0}],
                 "changes": [{"kind": "format", "from": "CAT062", "to": "CAT048 plots + CAT034 status (a radar)",
                              "why": "reshape in the profile: the receiver's display takes this shape"},
                             {"kind": "identity", "from": {"sac": 9, "sic": 17}, "to": {"sac": 60, "sic": 7},
                              "why": "the identity agreed with the receiver is stamped on every record"}],
                 "unchanged": False}]

OPENAPI = {
    "openapi": "3.0.3",
    "info": {"title": "Boundary monitor API", "version": "1",
             "description": "Read-only status of the boundary services on this node (schema "
                            "goat.boundary-status/v1). Reachable on the goatnet mesh, and on the tunnel address for the receiver "
                            "when monitor.receiver is declared. No request changes "
                            "anything; there is no write endpoint."},
    "servers": [{"url": "/"}],
    "paths": {
        "/api/v1/status": {"get": {
            "summary": "Every boundary service on this node", "operationId": "status",
            "description": "One document per service: receiver, uptime, and per feed its state, totals, "
                           "1 m / 15 m / 1 h / 24 h windows, per-minute counts and acknowledgement. "
                           "`stale` is true when the service has not written for 30 s; its feeds then read `stopped`. "
                           "On an egress relay (`input: relay`) each feed also carries `relay` and counts what the "
                           "inside sender sent (`received`), what was refused (`rejected_source`, `rejected_format`), "
                           "what was held because no receiver is declared (`held`) and what was forwarded (`datagrams`).",
            "responses": {"200": {"description": "The status documents",
                                  "content": {_J: {"example": _STATUS_EXAMPLE}}}}}},
        "/api/v1/feeds/{feed}": {"get": {
            "summary": "One feed", "operationId": "feed",
            "parameters": [{"name": "feed", "in": "path", "required": True, "schema": {"type": "string"},
                            "example": "tpq53", "description": "The feed name from the profiles file"}],
            "responses": {"200": {"description": "The feed", "content": {_J: {"example": _FEED_EXAMPLE}}},
                          "404": {"description": "No feed by that name",
                                  "content": {_J: {"example": {"error": "no such feed"}}}}}}},
        "/api/v1/feedback": {"get": {
            "summary": "What the receiver has told us", "operationId": "feedback",
            "description": "goat.bn-feedback/v1 messages accepted from the receiver's address: hello, ack "
                           "(datagrams received per SIC in a window), status and request. Messages never change "
                           "what is sent.",
            "responses": {"200": {"description": "One entry per service",
                                  "content": {_J: {"example": [{"node": "boundary-host-a", **_FEEDBACK_EXAMPLE}]}}}}}},
        "/api/v1/sim-status": {"get": {
            "summary": "Real or simulated, per feed, declared out of band", "operationId": "simStatus",
            "description": "Some sources can run a simulation and send it in the same form as real data, with no "
                           "flag in the data. For each feed: `declared` (real, simulated or unknown), `basis` "
                           "(`profile` when nobody has declared anything, `operator` when someone ran "
                           "simple-bn-sim-status on the node, with `set_by`, `set_at` and `note`), whether the "
                           "source can simulate and whether it marks it, the SIM bit this node sends (`null` when "
                           "records pass unchanged) and whether the two agree. The declaration never changes what "
                           "is sent.",
            "responses": {"200": {"description": "One entry per feed",
                                  "content": {_J: {"example": _SIM_EXAMPLE}}}}}},
        "/api/v1/modifications": {"get": {
            "summary": "What this node changes in each feed", "operationId": "modifications",
            "description": "Per feed, measured from the ASTERIX blocks themselves: `input` is each SAC/SIC that "
                           "arrived from upstream with its categories and count (empty for a feed generated on this "
                           "node), `output` is each category and SAC/SIC sent to the receiver with its count, and "
                           "`changes` lists what this node did between the two: `format` (a reshape, for example "
                           "CAT062 into CAT048 plots and CAT034 status), `identity` (the SAC/SIC agreed with the "
                           "receiver stamped on every record, from and to) and `sim-bit`. `unchanged` is true when a "
                           "relayed feed leaves exactly as it arrived.",
            "responses": {"200": {"description": "One entry per feed",
                                  "content": {_J: {"example": _MOD_EXAMPLE}}}}}},
        "/api/v1/health": {"get": {
            "summary": "Is every feed sending?", "operationId": "health",
            "description": "For scripts and probes: 200 when every feed of every service is sending (for an "
                           "egress relay with no receiver declared: holding what the inside sender sends) and the "
                           "status files are fresh, 503 otherwise with the problems.",
            "responses": {"200": {"description": "Healthy", "content": {_J: {"example": {"ok": True, "problems": []}}}},
                          "503": {"description": "A feed is not sending, or a status file is stale",
                                  "content": {_J: {"example": {"ok": False,
                                                               "problems": ["boundary-host-a/tpq53:receiver-refusing"]}}}}}}},
    },
}

# Swagger UI from a pinned version, integrity-checked; the spec and every call stay on this host.
DOCS = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Boundary monitor API</title>
<link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css"
 integrity="sha384-wxLW6kwyHktdDGr6Pv1zgm/VGJh99lfUbzSn6HNHBENZlCN7W602k9VkGdxuFvPn" crossorigin="anonymous">
<style>body{margin:0}.topbar{display:none}</style></head><body>
<p style="font:14px system-ui,sans-serif;margin:12px 16px"><a href="/">&larr; the monitor page</a></p>
<div id="ui"></div>
<script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"
 integrity="sha384-wmyclcVGX/WhUkdkATwhaK1X1JtiNrr2EoYJ+diV3vj4v6OC5yCeSu+yW13SYJep" crossorigin="anonymous"></script>
<script>SwaggerUIBundle({url:"/api/v1/openapi.json",dom_id:"#ui",tryItOutEnabled:true,
 displayRequestDuration:true,defaultModelsExpandDepth:-1,docExpansion:"list"});</script>
</body></html>"""


def load(status_dir):
    out, now = [], time.time()
    for p in sorted(glob.glob(os.path.join(status_dir, "*.json"))):
        try:
            with open(p) as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue
        d["stale"] = now - d.get("now", 0) > 30
        if d["stale"]:
            for fd in d.get("feeds", []):
                fd["state"] = "stopped"
        out.append(d)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bind", required=True, help="mesh address to bind")
    ap.add_argument("--port", type=int, default=8740)
    ap.add_argument("--status-dir", default="/run/simple-bn")
    a = ap.parse_args()

    class H(BaseHTTPRequestHandler):
        def _send(self, code, ctype, body):
            b = body.encode() if isinstance(body, str) else body
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(b)))
            self.end_headers()
            self.wfile.write(b)

        def do_GET(self):
            path = self.path.split("?")[0]
            if path in ("/", "/index.html"):
                self._send(200, "text/html; charset=utf-8", PAGE)
            elif path in ("/api/status", "/api/v1/status"):
                self._send(200, "application/json", json.dumps(load(a.status_dir)))
            elif path == "/api/v1/health":
                # 200 when every feed of every service is sending (or, for a relay with no receiver
                # declared, holding what it receives) and the status files are fresh; 503 otherwise.
                docs = load(a.status_dir)
                bad = [f"{d.get('node')}/{f['feed']}:{f['state']}" for d in docs for f in d.get("feeds", [])
                       if f["state"] not in ("sending", "holding")] + [f"{d.get('node')}:stale" for d in docs if d.get("stale")]
                ok = bool(docs) and not bad
                self._send(200 if ok else 503, "application/json", json.dumps({"ok": ok, "problems": bad or ([] if docs else ["no status"])}))
            elif path.startswith("/api/v1/feeds/"):
                name = path.rsplit("/", 1)[1]
                hit = [f for d in load(a.status_dir) for f in d.get("feeds", []) if f["feed"] == name]
                self._send(200 if hit else 404, "application/json", json.dumps(hit[0] if hit else {"error": "no such feed"}))
            elif path == "/api/v1/sim-status":
                self._send(200, "application/json", json.dumps(
                    [{"node": d.get("node"), "feed": f["feed"], "title": f.get("title"), "sac": f.get("sac"),
                      "sic": f.get("sic"), **(f.get("sim_status") or {"declared": None, "basis": None})}
                     for d in load(a.status_dir) for f in d.get("feeds", [])]))
            elif path == "/api/v1/modifications":
                self._send(200, "application/json", json.dumps(
                    [{"node": d.get("node"), "feed": f["feed"], "title": f.get("title"), "state": f.get("state"),
                      **(f.get("modifications") or {"source": None, "input": [], "output": [], "changes": [], "unchanged": None})}
                     for d in load(a.status_dir) for f in d.get("feeds", [])]))
            elif path == "/api/v1/feedback":
                self._send(200, "application/json", json.dumps([{"node": d.get("node"), **(d.get("feedback") or {})} for d in load(a.status_dir)]))
            elif path == "/api/v1/openapi.json":
                self._send(200, "application/json", json.dumps(OPENAPI))
            elif path in ("/api/docs", "/api/docs/"):
                self._send(200, "text/html; charset=utf-8", DOCS)
            elif path == "/favicon.ico":
                self._send(204, "image/x-icon", b"")
            elif path == "/healthz":
                self._send(200, "text/plain", "ok\n")
            else:
                self._send(404, "text/plain", "not found\n")

        def log_message(self, *args):
            pass

    ThreadingHTTPServer((a.bind, a.port), H).serve_forever()


if __name__ == "__main__":
    main()
