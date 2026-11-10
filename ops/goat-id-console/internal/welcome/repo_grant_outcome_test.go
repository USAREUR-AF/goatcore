package welcome

import (
	"strings"
	"testing"
)

// #2107: the welcome package listed every ATTEMPTED grant under "You've been
// added", including the ones GitHub refused. These pin the truthful split.

func devWithGrants(grants ...RepoGrant) Recipient {
	return Recipient{Name: "Dev", Handle: "dev", Tier: "developer",
		Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true,
		GitHubUsername: "octocat", RepoGrants: grants}
}

// repoSection returns just the "Repository access" section of a rendered
// package (heading through the line before the next section's rule), so an
// assertion cannot pass on text from another section.
func repoSection(t *testing.T, out string) string {
	t.Helper()
	const rule = "\n============================================================"
	i := strings.Index(out, "Repository access")
	if i < 0 {
		t.Fatalf("no Repository access section:\n%s", out)
	}
	rest := out[i:]
	// Skip the rule under the heading, then stop at the next section's rule.
	head := strings.Index(rest, rule)
	if head < 0 {
		return rest
	}
	body := rest[head+len(rule):]
	if j := strings.Index(body, rule); j >= 0 {
		return rest[:head+len(rule)+j]
	}
	return rest
}

func TestRender_mixedRepoGrantOutcomesSplitTruthfully(t *testing.T) {
	out := Render(zagrosCatalog(), devWithGrants(
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-zenoh-patterns", Permission: "pull", Outcome: "invited", Source: "policy"},
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-time-hygiene", Permission: "pull", Outcome: "failed", Source: "policy", HTTPStatus: 404, Failure: "repo_not_visible_to_token"},
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-external-service", Permission: "pull", Outcome: "failed", Source: "policy", HTTPStatus: 403, Failure: "token_forbidden"},
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-mesh-change-control", Permission: "pull", Outcome: "failed", Source: "policy"},
	), Artifacts{SetupKey: "K"})
	sec := repoSection(t, out)

	added := strings.Index(sec, "You've been added to these repositories")
	notGranted := strings.Index(sec, "has not been confirmed")
	if added < 0 || notGranted < 0 || notGranted < added {
		t.Fatalf("want the success list, then the not-granted list:\n%s", sec)
	}
	successPart, failedPart := sec[added:notGranted], sec[notGranted:]
	if !strings.Contains(successPart, "example-org/hello-galaxy-zenoh-patterns") {
		t.Errorf("the invited repo is missing from the success list:\n%s", successPart)
	}
	for _, repo := range []string{"hello-galaxy-time-hygiene", "hello-galaxy-external-service", "hello-galaxy-mesh-change-control"} {
		if strings.Contains(successPart, repo) {
			t.Errorf("failed grant %s is presented as added:\n%s", repo, successPart)
		}
		if !strings.Contains(failedPart, "example-org/"+repo) {
			t.Errorf("failed grant %s is not named as not granted:\n%s", repo, failedPart)
		}
	}
	if strings.Contains(failedPart, "zenoh-patterns") {
		t.Errorf("the invited repo is listed as failed:\n%s", failedPart)
	}
	if !strings.Contains(failedPart, "sponsor") {
		t.Errorf("the not-granted list gives no next step:\n%s", failedPart)
	}
}

func TestRender_allRepoGrantsFailedMakesNoSuccessClaim(t *testing.T) {
	out := Render(zagrosCatalog(), devWithGrants(
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-time-hygiene", Permission: "pull", Outcome: "failed"},
		RepoGrant{Owner: "example-org", Repo: "hello-galaxy-external-service", Permission: "pull", Outcome: "failed"},
	), Artifacts{SetupKey: "K"})
	sec := repoSection(t, out)
	for _, claim := range []string{"You've been added", "accept the invite", "pending invitation"} {
		if strings.Contains(sec, claim) {
			t.Errorf("every grant failed, yet the section says %q:\n%s", claim, sec)
		}
	}
	if !strings.Contains(sec, "has not been confirmed") ||
		!strings.Contains(sec, "example-org/hello-galaxy-time-hygiene") ||
		!strings.Contains(sec, "example-org/hello-galaxy-external-service") {
		t.Errorf("the failed grants are not named as not granted:\n%s", sec)
	}
}

// An outcome the renderer does not recognize (blank, or a future value) is not
// evidence of access, so it must not land on the success list either.
func TestRender_unknownRepoGrantOutcomeIsNotSuccess(t *testing.T) {
	out := Render(zagrosCatalog(), devWithGrants(
		RepoGrant{Owner: "example-org", Repo: "mystery", Permission: "pull", Outcome: ""},
	), Artifacts{SetupKey: "K"})
	if strings.Contains(repoSection(t, out), "You've been added") {
		t.Errorf("a grant with no recorded outcome is presented as added:\n%s", out)
	}
}

func TestRender_unconfirmedGrantDoesNotInventAnOutcome(t *testing.T) {
	for _, outcome := range []string{"", "future", "failed"} {
		t.Run(outcome, func(t *testing.T) {
			sec := repoSection(t, Render(zagrosCatalog(), devWithGrants(
				RepoGrant{Owner: "example", Repo: "unknown", Outcome: outcome, Failure: "network_error"},
			), Artifacts{SetupKey: "K"}))
			for _, claim := range []string{"no invitation is on its way", "Nothing is wrong with your GitHub account", "You've been added"} {
				if strings.Contains(sec, claim) {
					t.Errorf("unconfirmed outcome asserts %q: %s", claim, sec)
				}
			}
			if !strings.Contains(sec, "has not been confirmed") || !strings.Contains(sec, "/github") {
				t.Errorf("missing uncertainty or recovery step: %s", sec)
			}
		})
	}
}
