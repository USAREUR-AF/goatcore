#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Boundary monitor: a small read-only page over the status files a boundary node writes.

Reads every *.json in --status-dir (schema goat.boundary-status/v1), one per boundary
service on this node, and serves:
  GET /            the page (refreshes itself every 5 s)
  GET /api/status  the status documents as JSON
  GET /healthz     ok

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
.s-sending{background:var(--ok)}.s-stalled,.s-starting,.s-reconnecting{background:var(--warn)}.s-receiver-refusing,.s-send-error,.s-stopped{background:var(--bad)}
table{width:100%;border-collapse:collapse;margin-top:8px;font-variant-numeric:tabular-nums}
th,td{text-align:right;padding:3px 4px;border-bottom:1px solid var(--line)}th:first-child,td:first-child{text-align:left;color:var(--mute);font-weight:500}
th{color:var(--mute);font-weight:500;font-size:12px}
.kinds{margin-top:8px;display:flex;flex-wrap:wrap;gap:6px}.k{font-size:12px;border:1px solid var(--line);border-radius:6px;padding:1px 6px}
.ack{margin-top:8px;font-size:12px;color:var(--mute)}svg{display:block;margin-top:8px;width:100%;height:42px}
.node{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px;margin-bottom:12px;display:flex;flex-wrap:wrap;gap:18px}
.node b{display:block;font-size:12px;color:var(--mute);font-weight:500}
.err{color:var(--bad)}
</style></head><body><main>
<h1>Boundary monitor</h1><div class="sub" id="sub">loading…</div>
<div id="nodes"></div>
<script>
const W=["1m","15m","1h","24h"];
const fmt=n=>n==null?"–":n>=1e6?(n/1e6).toFixed(1)+"M":n>=1e4?(n/1e3).toFixed(1)+"k":String(n);
const ago=t=>t==null?"never":Math.max(0,Math.round(Date.now()/1000-t))+" s ago";
const dur=s=>{const d=Math.floor(s/86400),h=Math.floor(s%86400/3600),m=Math.floor(s%3600/60);return (d?d+"d ":"")+h+"h "+m+"m"};
function spark(pm,col){if(!pm.length)return"";const v=pm.map(x=>x[1]),mx=Math.max(1,...v),w=300,h=40,bw=w/v.length;
 const bars=v.map((y,i)=>{const bh=y?Math.max(1.5,(y/mx)*(h-4)):0;return `<rect x="${(i*bw+0.5).toFixed(1)}" y="${(h-bh).toFixed(1)}" width="${(bw-1).toFixed(1)}" height="${bh.toFixed(1)}" fill="${col}"/>`}).join("");
 return `<svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="datagrams per minute, last 60 minutes"><title>datagrams per minute, last 60 minutes (peak ${mx})</title><line x1="0" y1="${h-0.5}" x2="${w}" y2="${h-0.5}" stroke="var(--line)"/>${bars}</svg><div class="ack" style="margin-top:2px">datagrams per minute · last 60 min · peak ${mx}</div>`}
async function tick(){try{const r=await fetch("api/status",{cache:"no-store"});const docs=await r.json();
 document.getElementById("sub").textContent="Updated "+new Date().toLocaleTimeString()+" · "+docs.length+" boundary service(s) on this node";
 let html="";docs.forEach(d=>{
  html+=`<div class="node"><div><b>Node</b>${d.node} · ${d.kind}</div><div><b>Service</b>${d.service}</div><div><b>Uptime</b>${dur(d.uptime_s)}</div>
  <div><b>Receiver</b>${d.receiver.label}</div><div><b>Transport</b>${d.receiver.transport} · ${d.receiver.format}</div><div><b>Encryption</b>${d.receiver.encryption}</div>
  ${d.stale?'<div class="err"><b>Status file</b>stale: service not writing</div>':""}</div><div class="grid">`;
  d.feeds.forEach((f,i)=>{const col=`var(--f${i%4})`;
   const rows=[["Datagrams","datagrams"],["Detections","detections"],["Tracks","tracks"],["Service msgs","cat034"],["Bytes","bytes"],["Send errors","send_errors"],["Refused","refused"],["Dropped","dropped"]];
   html+=`<div class="card" style="--c:${col}"><div class="hd"><div><div class="name">${f.title}</div>
   <div class="ids">SAC ${f.sac} · SIC ${f.sic} · src port ${f.src_port} · every ${f.update_period_s}s${f.transport?' · '+f.transport:''}</div></div>
   <span class="badge s-${f.state}">${f.state}</span></div>
   <table><tr><th></th>${W.map(w=>`<th>${w}</th>`).join("")}<th>total</th></tr>
   ${rows.map(([l,k])=>`<tr><td>${l}</td>${W.map(w=>`<td>${fmt((f.windows[w]||{})[k]||0)}</td>`).join("")}<td>${fmt(f.totals[k]||0)}</td></tr>`).join("")}</table>
   ${spark(f.per_minute,col)}
   <div class="kinds">${Object.entries(f.live_targets_by_kind).map(([k,n])=>`<span class="k">${k}: ${n}</span>`).join("")||'<span class="k">no live targets</span>'}</div>
   <div class="ack">Last send ${ago(f.last_send)} · Ack: ${f.ack.mode==="none"?f.ack.note:f.ack.mode}${f.ack.last_refused?" · last refused "+ago(f.ack.last_refused):""}
   ${f.last_error?`<br><span class="err">Last error: ${f.last_error}</span>`:""}</div></div>`});
  html+="</div>"});
 document.getElementById("nodes").innerHTML=html||"<p>No boundary service status found.</p>";
}catch(e){document.getElementById("sub").textContent="Monitor API unreachable: "+e}}
tick();setInterval(tick,5000);
</script></main></body></html>"""


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
            elif path == "/api/status":
                self._send(200, "application/json", json.dumps(load(a.status_dir)))
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
