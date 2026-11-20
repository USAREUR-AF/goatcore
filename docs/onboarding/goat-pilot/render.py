"""Build the standalone, offline HTML edition from bank.json."""
from pathlib import Path
from html import escape as e
from collections import Counter
import re

from pilot import ROOT, BANK, LABEL, link

STYLE = """
:root{font-family:system-ui,sans-serif;color:#172c35;background:#f2f6f4;font-size:16px}*{box-sizing:border-box}
body{margin:0}main{max-width:980px;margin:auto;padding:34px 22px 80px}header{border-top:5px solid #1e6859;padding-top:18px}
h1{font-size:clamp(30px,5vw,49px);letter-spacing:-.04em;margin:.2em 0}header p{max-width:78ch;line-height:1.55}
h2{font-size:20px;margin:28px 0 10px}input,select{font:inherit;background:white;border:1px solid #a9b8af;border-radius:5px;padding:11px 12px;width:100%}
label{font-size:14px;font-weight:650;display:grid;gap:5px}.controls{display:grid;grid-template-columns:2fr 1fr;gap:13px}
details{background:white;border:1px solid #d0dcd4;border-radius:6px;margin:7px 0}summary{padding:14px 15px;cursor:pointer;font-weight:580;line-height:1.5}
small{color:#406457;margin-right:.7em}article{padding:0 18px 14px 27px;line-height:1.55}.flag{font-size:13px;font-weight:bold;color:#23634c}
a{color:#195a77;overflow-wrap:anywhere}.links{font-size:14px}.count{font-size:14px;color:#425850}footer{margin-top:32px;font-size:13px}
.notice{background:#fff4d6;border-left:4px solid #b57a00;padding:10px 12px;line-height:1.45}
.section-intro{background:white;border:1px solid #a9b8af;border-radius:6px;padding:2px 16px;margin:12px 0 14px;line-height:1.55}
.help-link{display:inline-block;background:#1e6859;color:white;border-radius:5px;padding:10px 14px;font-weight:700;text-decoration:none}
.help-panel{background:white;border:1px solid #a9b8af;border-radius:6px;margin:20px 0;padding:4px 18px 16px;line-height:1.55}
@media(max-width:600px){.controls{grid-template-columns:1fr}main{padding:18px 14px}}
"""
SCRIPT = """
const input=document.querySelector('#find'),filter=document.querySelector('#type');
function refresh(){
 const terms=input.value.toLowerCase().trim().split(/\\s+/).filter(Boolean);let count=0;
 document.querySelectorAll('section').forEach(s=>{let n=0;
 s.querySelectorAll('details').forEach(d=>{const ok=(!filter.value||filter.value===d.dataset.kind)&&terms.every(t=>(s.querySelector('h2').textContent+' '+d.textContent).toLowerCase().includes(t));
 d.hidden=!ok;if(ok)n++;});s.hidden=n===0;count+=n;});
 document.querySelector('#count').textContent=count+' of 72 questions';
}
input.addEventListener('input',refresh);filter.addEventListener('change',refresh);refresh();
"""

def render():
    help_info=BANK["help"]
    out=['<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>GOAT onboarding reference pilot</title><style>'+STYLE+'</style><main><header><h1>GOAT onboarding questions</h1><p>Open a question for a plain-English response. This offline edition uses development documentation captured on 20 November 2026. The companion WSL pilot can check a selected Git revision for updates. An installed release or a local authority may give a different answer.</p><a class="help-link" href="#ask-for-help">'+e(help_info["label"])+"</a></header>"]
    out.append('<aside class="help-panel" id="ask-for-help"><h2>Ask for help</h2><p>Any available '+e(help_info["team"])+' member can guide you or privately connect you with whoever can help. You do not need an individual’s name. Team routing does not grant approval or certificate-issuance authority.</p>')
    if help_info["destination"]:
        out.append('<p><a class="help-link" href="'+e(help_info["destination"])+ '">Open the monitored help destination</a></p>')
    else:
        out.append('<p class="notice">'+e(help_info["pending"])+" This known gap remains until the path is confirmed and verified.</p>")
    out.append('</aside>')
    out.append('<div class="controls"><label>Find a question<input id="find" type="search" placeholder="Try fingerprint, sandbox, or Q23"></label><label>Answer type<select id="type"><option value="">All answer types</option>')
    out.extend('<option value="'+k+'">'+e(v)+'</option>' for k,v in LABEL.items())
    out.append('</select></label></div><p class="count" aria-live="polite" id="count">72 questions</p>')
    last=None
    for q in BANK["questions"]:
        if last!=q["step"]:
            if last is not None: out.append('</section>')
            last=q["step"];out.append('<section><h2>'+e(last)+'</h2>')
            if int(q["id"][1:]) >= 49:
                out.append('<p class="notice">'+e(BANK["federation_notice"])+"</p>")
            if q["id"] == "Q49":
                intro=BANK["federation_intro"]
                out.append('<aside class="section-intro"><p><strong>Example newcomer question:</strong> '+e(intro["question"])+
                           '</p><p>'+e(intro["guidance"])+"</p></aside>")
        out.append('<details data-kind="'+q["kind"]+'"><summary><small>'+q["id"]+'</small>'+e(q["question"])+'</summary><article><span class="flag">'+LABEL[q["kind"]]+'</span><p>'+e(q["answer"])+'</p>')
        for r in q["refs"]:
            s=BANK["sources"][r["key"]]
            out.append('<div class="links"><a href="'+e(link(s,r["line"]))+'">'+e(s["path"].split('/')[-1]+' · '+r["heading"].strip('`'))+'</a> · '+s["commit"][:8]+'</div>')
        out.append('</article></details>')
    out.append('</section><footer>Reviewed pilot · Pre-access help destination pending CODEOWNER confirmation. See <a href="LOCAL_CONTACTS.md">local contact routing</a>. Source links point to the exact revisions used for this edition.</footer></main><script>'+SCRIPT+'</script></html>')
    (ROOT/'GOAT_Onboarding_72.html').write_text(''.join(out),encoding="utf-8")


if __name__ == "__main__":
    render()
