"""Free-text retrieval smoke cases; target is a document among the top five."""
from pilot import BANK, bundled, search
CASES = [
    ("which file signs my cbor identity bundle","start"),
    ("PowerShell GOAT executable verification checksum","install"),
    ("why did the sandbox say round trip complete","start"),
    ("rejected bearer token in the portal","trouble"),
    ("how do I resume my lessons in the tutor","start"),
    ("what does a subscription watch","use"),
    ("what can partners read with just a trust bundle","boundary"),
    ("which verification keys do the federation operators read aloud","run"),
    ("who owns the certificate authority when two separate sovereign networks connect","boundary"),
    ("retiring the old certificate authority after an issuer change","rotate"),
    ("what does Panoscope show","tools"),
    ("which tag picks the latest whole repository release","versions"),
]

# These deliberately avoid the wording in the prepared question bank. They cover
# common novice paraphrases and the trust-sensitive federation material.
FRESH_CASES = [
    ("why does the shell not find goat after download", "install"),
    ("how can I resume the tutorial from my last lesson", "start"),
    ("the downloaded hash is different from the published value", "install"),
    ("why must a counterparty CA stay on a dedicated edge instead of the core router", "boundary"),
    ("how do operators authenticate federation material out of band", "run"),
    ("can an announced replacement issuer be trusted before the call", "rotate"),
    ("what happens to traffic when a version one peering changes issuers", "rotate"),
    ("should old trust overlap when its private key may be stolen", "rotate"),
]

if __name__ == "__main__":
    files = bundled()["files"]
    passed = 0
    for query, key in CASES:
        target = BANK["sources"][key]
        hits = search(query,None,files)
        ok = any(d["repo"] == target["repo"] and d["path"] == target["path"] for _,d,_,_ in hits)
        passed += int(ok)
        print("HIT " if ok else "MISS", query)
    print(f"{passed}/{len(CASES)} expected documents in top five; development cases, no model grading.")
    fresh_passed = 0
    for query, key in FRESH_CASES:
        target = BANK["sources"][key]
        hits = search(query,None,files)
        ok = any(d["repo"] == target["repo"] and d["path"] == target["path"] for _,d,_,_ in hits)
        fresh_passed += int(ok)
        print("DEVELOPMENT PARAPHRASE HIT " if ok else "DEVELOPMENT PARAPHRASE MISS", query)
    print(f"{fresh_passed}/{len(FRESH_CASES)} development paraphrases retrieved the expected document in the top five; these are not held-out learner questions.")
    matched, tested, misses = 0, 0, []
    for q in BANK["questions"]:
        if not q["refs"]:
            continue
        keys = [q["retrieval_key"]] if q.get("retrieval_key") else [r["key"] for r in q["refs"]]
        expected = {(BANK["sources"][key]["repo"],BANK["sources"][key]["path"]) for key in keys}
        hits = search(q["question"], None, files)
        ok = any((d["repo"],d["path"]) in expected for _,d,_,_ in hits)
        matched += int(ok)
        tested += 1
        if not ok:
            misses.append(q["id"])
    print(f"DEVELOPMENT RETRIEVAL RESULT: {matched}/{tested} source-backed bank questions retrieved a mapped document in the top five without ID or heading hints.")
    print("Known misses:", ", ".join(misses) or "none")
    print("Answer accuracy with new users is unmeasured.")
