"""Reproducible acceptance checks. Operates on temporary local Git repositories."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from pilot import BANK, QUESTIONS, ROOT, answer, blob, bundled, terms, search


def cli(args, cache, success=True):
    r = subprocess.run([sys.executable, str(ROOT/"pilot.py"), *args], env=dict(os.environ,XDG_CACHE_HOME=str(cache.parent)),
                       text=True, capture_output=True)
    assert (r.returncode==0)==success,(args,r.stderr)
    return r.stdout if success else r.stderr


def git(root, *args):
    subprocess.run(["git","-C",str(root),*args],capture_output=True,check=True)


def commit(root):
    git(root,"add","-A")
    git(root,"-c","user.name=Fixture","-c","user.email=fixture@example.invalid","commit","-qm","test revision")


def check():
    view = bundled()
    assert list(QUESTIONS) == [f"Q{i:02d}" for i in range(1,73)]
    help_info=BANK["help"]
    assert help_info["label"] == "Ask for help"
    assert help_info["team"] == "GOAT onboarding or support team"
    if help_info["destination"] is None:
        assert "GOAT access" in help_info["pending"] and "CODEOWNER" in help_info["pending"]
    else:
        assert help_info["destination"].startswith("https://")
    assert all(text in QUESTIONS["Q01"]["answer"] for text in
               ("Any available GOAT onboarding or support team member", "privately connect",
                "before GOAT access", "only the responsible authority"))
    intro=BANK["federation_intro"]
    assert intro["question"] == ("Our team wants to connect its GOAT environment to a partner’s. "
                                 "Where do we start with federation, which certificates do we need, and who can help?")
    assert all(text in intro["guidance"] for text in
               ("Any available GOAT team member", "privately connect", "without GOAT access",
                "CODEOWNER confirmation and verification"))
    q49=QUESTIONS["Q49"]
    assert all(text in q49["answer"] for text in
               ("operates its own CA", "platform operators coordinate", "authorized to approve"))
    html=(ROOT/"GOAT_Onboarding_72.html").read_text(encoding="utf-8")
    assert html.index(intro["question"]) < html.index("<small>Q49</small>")
    assert intro["guidance"] in html
    with tempfile.TemporaryDirectory() as tmp:
        base=Path(tmp); cache=base/"cache-home"/"goat-onboarding-pilot"
        for q in QUESTIONS.values():
            response=cli(["ask",q["id"],"--bundled"], cache)
            assert q["question"] in response and q["answer"] in response
            assert bool(q["refs"]) == (q["kind"] != "L")
            for r in q["refs"]:
                src=BANK["sources"][r["key"]]
                doc=next(d for d in view["files"] if d["repo"]==src["repo"] and d["path"]==src["path"])
                assert doc["text"].splitlines()[r["line"]-1].lstrip("# ")==r["heading"]
                assert "?plain=1#L" in response
            assert answer(q,view["files"]) is not None
        print("BUNDLED CARD EXECUTION: 72/72 cards returned their stored answer; citation anchors resolve to named headings. This is not an answer-accuracy result.")
        print("PASS: Ask for help routes to a GOAT onboarding or support team; the pre-GOAT monitored destination remains an explicit known gap pending CODEOWNER confirmation")
        q63=QUESTIONS["Q63"]
        assert all(text in q63["answer"] for text in
                   ("PEERING-EDGE", "goat_zenoh_edge_partner_ca_path", "never the core router", "mentor"))
        assert any(r["key"]=="run" and r["line"]==544 for r in q63["refs"])
        q68=QUESTIONS["Q68"]
        assert all(text in q68["answer"] for text in
                   ("cannot be waived", "every required check must report PASS", "rerun", "both operators", "`exchanging`"))
        assert {("run",58),("run",286)} <= {(r["key"],r["line"]) for r in q68["refs"]}
        q72=QUESTIONS["Q72"]
        ordered=("cuts that partner from the tunnel", "remove the partner trust domain", "remove the partner CA",
                 "restart the edge router", "remove the core peering entry")
        positions=[q72["answer"].index(text) for text in ordered]
        assert positions == sorted(positions)
        assert all(text in q72["answer"] for text in
                   ("Counterpart notification", "cited sources do not say", "ask a mentor", "do not delay"))
        assert any(r["key"]=="run" and r["line"]==778 for r in q72["refs"])
        q=QUESTIONS["Q23"]
        same=[dict(d,commit="f"*40) for d in view["files"]]
        canned=answer(q,same)
        assert canned is not None and "/blob/ffffffffffffffffffffffffffffffffffffffff/" in canned[1][0]
        changed=[]
        target=BANK["sources"][q["refs"][0]["key"]]
        for d in same:
            if (d["repo"],d["path"]) == (target["repo"],target["path"]):
                text=d["text"]+"\nchanged guidance\n"
                d=dict(d,text=text,blob=blob(text))
            changed.append(d)
        assert answer(q,changed) is None
        assert answer(q,[d for d in same if (d["repo"],d["path"]) !=
                      (target["repo"],target["path"])]) is None
        assert answer(QUESTIONS["Q70"],same) is None
        typed=QUESTIONS["Q63"]["question"].replace("’", "'")
        assert QUESTIONS["Q63"]["answer"] in cli(["ask",typed,"--bundled"],cache)
        print("PASS: unchanged source remains usable at a newer commit; changed or missing source withholds the prepared answer")
        repos=[base/"core",base/"stuff"]
        for root in repos:
            root.mkdir();git(root,"init","-q","-b","main")
            (root/"docs").mkdir();(root/"README.md").write_text("# Fixture\n")
        old=repos[0]/"docs"/"old.md"
        old.write_text("# Lighthouse widget\nThe reference colour is amber.\n")
        for root in repos: commit(root)
        opts=["--core",str(repos[0]),"--stuff",str(repos[1])]
        query=["ask","lighthouse widget","--live",*opts]
        assert "amber" in cli(query,cache)
        old.write_text("# Lighthouse widget\nThe reference colour is violet.\n")
        commit(repos[0]);t=cli(query,cache);assert "violet" in t and "amber" not in t
        dest=old.with_name("moved.md");old.rename(dest);commit(repos[0]);t=cli(query,cache)
        assert "moved.md" in t and "old.md" not in t
        dest.unlink();commit(repos[0]);assert "No supporting passage" in cli(query,cache)
        (repos[0]/"docs"/"new.md").write_text("# Lighthouse widget\nThe reference colour is green.\n")
        commit(repos[0]);assert "green" in cli(query,cache)
        t=cli(["ask","Q23",*opts,"--live"],cache)
        assert QUESTIONS["Q23"]["answer"] not in t
        before=(cache/"index.json").read_bytes()
        cli([*query,"--stuff-ref","missing-ref"],cache,success=False)
        assert (cache/"index.json").read_bytes()==before
        t=cli(["ask","Q51"],cache)
        assert (QUESTIONS["Q51"]["answer"] in t and "Local team needed" in t and
                "Cached snapshot" in t)
        print("PASS: manual sync makes changed, renamed and newly added docs immediately searchable, removes deleted docs, and withholds stale prepared answers")
        print("PASS: failed two-repo sync preserves previous index; named-authority question escalates")
        print("Limits: these checks test delivery, citations and refresh behavior. Human answer accuracy, live model behavior and learner outcomes require separate review.")

if __name__=="__main__":
    check()
