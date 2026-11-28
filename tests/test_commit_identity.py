"""Tests for .github/commit-identity/check.py (ADR 1276)."""
import importlib.util
import sys
from pathlib import Path

def A(text: str) -> str:
    """An address written as `local at domain`, so no line here holds a mailbox."""
    return text.replace(" at ", "@")


ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("commit_identity", ROOT / ".github/commit-identity/check.py")
ci = importlib.util.module_from_spec(_spec)
sys.modules["commit_identity"] = ci  # dataclasses look the module up by name
_spec.loader.exec_module(ci)

NOREPLY = A("12345678+octocat at users.noreply.github.com")
PERSONAL = A("octo.cat at mail.nonexistent")
OTHER = A("someone.else at mail.nonexistent")


def commit(**kw):
    base = dict(sha="a" * 40, author_email=NOREPLY, author_name="Octo Cat", author_login="octocat",
                committer_email=A("noreply at github.com"), committer_name="GitHub", committer_login="web-flow",
                message="fix: a thing\n")
    base.update(kw)
    return ci.Commit(**base)


# ---- the address rule


def test_noreply_and_github_own_addresses_are_acceptable():
    assert ci.is_acceptable(NOREPLY)
    assert ci.is_acceptable(A("octocat at users.noreply.github.com"))
    assert ci.is_acceptable(A("49699333+dependabot[bot] at users.noreply.github.com"))
    assert ci.is_acceptable(A("noreply at github.com"))
    assert ci.is_acceptable(A("support at github.com"))


def test_reserved_example_domains_are_acceptable():
    for a in (A("noreply at project.invalid"), A("a at example.com"), A("a at docs.example.org"), A("a at host.test"), A("a at x.example")):
        assert ci.is_acceptable(a), a


def test_every_other_mailbox_is_personal_whatever_the_provider():
    for a in (PERSONAL, A("a at gmail.com"), A("a at agency.gov"), A("a at unit.mil"), A("a at company.io"),
              A("someone at github.com"), A("a at users.noreply.github.com.evil.io"), A("a at notexample.com"),
              A("a at noreply.github.com")):
        assert not ci.is_acceptable(a), a


def test_case_and_whitespace_do_not_matter():
    assert ci.is_acceptable("  " + A("NoReply at GitHub.com") + " ")
    assert not ci.is_acceptable(" " + A("A at Gmail.COM") + " ")


def test_a_string_that_is_not_a_mailbox_is_not_a_finding():
    assert ci.is_acceptable("")
    assert ci.is_acceptable("not-an-address")
    assert ci.is_acceptable(A(" at nowhere"))


def test_allow_list_takes_whole_addresses_and_domains():
    allowed = ci.load_allowed("# placeholders\n" + A("release-bot at project.org") + "\n" + A(" at lists.project.org").strip() + "\n")
    assert ci.is_acceptable(A("Release-Bot at project.org"), allowed)
    assert ci.is_acceptable(A("dev at lists.project.org"), allowed)
    assert not ci.is_acceptable(A("person at project.org"), allowed)


def test_mask_never_shows_the_mailbox():
    m = ci.mask(A("octo.cat at mailprovider.com"))
    assert m == A("o*** at m***.com")
    assert "octo.cat" not in m and "mailprovider" not in m
    assert ci.mask("garbage") == "***"


# ---- what a commit carries, and whose it is


def test_clean_commit_has_no_finding():
    assert ci.findings_of([commit(message=f"fix: x\n\nSigned-off-by: octocat <{NOREPLY}>\n")]) == []


def test_signoff_with_personal_mailbox_belongs_to_the_author_by_login_or_name():
    by_login = ci.findings_of([commit(message=f"fix: x\n\nSigned-off-by: octocat <{PERSONAL}>\n")])
    by_name = ci.findings_of([commit(message=f"fix: x\n\nsigned-off-by: OCTO CAT <{PERSONAL}>\n")])
    assert [(f.where, f.owner) for f in by_login] == [("Signed-off-by", "octocat")]
    assert [(f.where, f.owner) for f in by_name] == [("Signed-off-by", "octocat")]


def test_author_and_committer_fields_belong_to_the_accounts_github_maps():
    c = commit(author_email=PERSONAL, committer_email=OTHER, committer_login="hubot")
    got = {(f.where, f.owner) for f in ci.findings_of([c])}
    assert got == {("author", "octocat"), ("committer", "hubot")}


def test_same_address_in_author_and_committer_is_one_finding():
    c = commit(author_email=PERSONAL, committer_email=PERSONAL, committer_login="octocat")
    assert [(f.where, f.owner) for f in ci.findings_of([c])] == [("author", "octocat")]


def test_commit_github_maps_to_no_account_has_no_owner():
    c = commit(author_email=PERSONAL, author_login=None)
    assert [(f.where, f.owner) for f in ci.findings_of([c])] == [("author", None)]


def test_trailer_matching_the_author_address_belongs_to_the_author():
    c = commit(author_email=PERSONAL, message=f"x\n\nSigned-off-by: Anything <{PERSONAL.upper()}>\n")
    assert {(f.where, f.owner) for f in ci.findings_of([c])} == {("author", "octocat"), ("Signed-off-by", "octocat")}


def test_someone_elses_trailer_has_no_owner():
    msg = f"x\n\nCo-authored-by: Someone Else <{OTHER}>\nSigned-off-by: Someone Else <{OTHER}>\n"
    got = ci.findings_of([commit(message=msg)])
    assert [(f.where, f.owner) for f in got] == [("Co-authored-by", None), ("Signed-off-by", None)]


def test_a_coauthor_line_is_never_the_authors_by_name_alone():
    got = ci.findings_of([commit(message=f"x\n\nCo-authored-by: octocat <{OTHER}>\n")])
    assert [(f.where, f.owner) for f in got] == [("Co-authored-by", None)]


def test_repeated_trailer_is_reported_once_per_commit():
    msg = f"x\n\nSigned-off-by: octocat <{PERSONAL}>\nSigned-off-by: octocat <{PERSONAL}>\n"
    assert len(ci.findings_of([commit(message=msg)])) == 1


def test_trailers_are_read_anywhere_in_the_message_and_tolerate_spacing():
    msg = f"x\n\nSigned-off-by:   octocat   <{PERSONAL}>  \n\nmore text\nCo-Authored-By: A <{OTHER}>\n"
    assert [t[0] for t in ci.trailers(msg)] == ["signed-off-by", "co-authored-by"]
    assert ci.trailers("Signed-off-by: no address here") == []


# ---- the phrase


def test_phrase_is_the_first_non_empty_line_exactly():
    assert ci.is_phrase("/email-ok", ci.ACK)
    assert ci.is_phrase("\n  /email-ok  \nthanks", ci.ACK)
    assert not ci.is_phrase("I think /email-ok is fine", ci.ACK)
    assert not ci.is_phrase("/email-ok-maintainer", ci.ACK)
    assert not ci.is_phrase("> /email-ok", ci.ACK)
    assert not ci.is_phrase("", ci.ACK)
    assert ci.is_phrase("/email-ok-maintainer", ci.OVERRIDE)


def test_only_the_persons_own_comment_counts():
    comments = [{"user": {"login": "hubot", "type": "User"}, "body": "/email-ok"},
                {"user": {"login": "octocat", "type": "User"}, "body": "what does /email-ok do?"},
                {"user": {"login": "octocat", "type": "Bot"}, "body": "/email-ok"}]
    assert not ci.own_ack_in(comments, "octocat")
    assert ci.own_ack_in(comments, "HUBOT")
    comments.append({"user": {"login": "OctoCat", "type": "User"}, "body": "/email-ok\n"})
    assert ci.own_ack_in(comments, "octocat")


# ---- the verdict


def _f(owner, where="Signed-off-by"):
    return ci.Finding("b" * 40, where, PERSONAL, owner)


def test_no_finding_passes():
    v = ci.decide([], set(), False)
    assert v.passed and ci.status_of(v)[0] == "success"


def test_first_time_blocks():
    v = ci.decide([_f("octocat")], set(), False)
    assert not v.passed and len(v.open) == 1 and ci.status_of(v)[0] == "failure"


def test_owners_acknowledgment_clears_their_own_findings_only():
    v = ci.decide([_f("octocat"), _f("hubot")], {"octocat"}, False)
    assert not v.passed
    assert [f.owner for f in v.open] == ["hubot"] and [f.owner for f in v.cleared] == ["octocat"]
    assert ci.decide([_f("OctoCat")], {"octocat"}, False).passed


def test_nobody_acknowledges_an_address_with_no_owner():
    v = ci.decide([_f(None, "Co-authored-by")], {"octocat", "hubot"}, False)
    assert not v.passed


def test_owner_override_clears_everything_for_this_pull_request():
    v = ci.decide([_f(None), _f("hubot")], set(), True)
    assert v.passed and v.overridden and len(v.cleared) == 2


# ---- what the person is told


def test_failure_comment_masks_and_gives_both_ways():
    v = ci.decide([_f("octocat"), _f(None, "Co-authored-by")], set(), False)
    body = ci.comment_body(v, pr_author="octocat", pr_author_id=12345678, base_ref="main", guide="https://guide.example/x")
    assert body.startswith(ci.MARK)
    assert PERSONAL not in body and ci.mask(PERSONAL) in body
    assert NOREPLY in body                       # the address to switch to, worked out for them
    assert "`/email-ok`" in body and "`/email-ok-maintainer`" in body
    assert "origin/main" in body and "--reset-author" in body
    assert "https://guide.example/x" in body
    assert A(" at octocat") in body


def test_cleared_comment_says_so_and_names_no_address():
    body = ci.comment_body(ci.decide([_f("octocat")], {"octocat"}, False),
                           pr_author="octocat", pr_author_id=1, base_ref="main", guide="g")
    assert "clear" in body and "***" not in body


def test_status_description_fits_githubs_limit():
    v = ci.decide([_f("octocat")] * 40, set(), False)
    assert len(ci.status_of(v)[1]) <= 140
