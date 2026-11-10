package handlers

// onboard_batch.go — bulk intake (ADR 1061 Phase F), machine-token realm. A file
// of many manifests, each linted independently; valid → inert proposals, invalid →
// per-record findings. ?dry_run validates without storing. Ported from the portal.

import (
	"encoding/json"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardlint"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"gopkg.in/yaml.v3"
)

type onboardBatchRequest struct {
	Content string `json:"content"`
	Format  string `json:"format,omitempty"`
	Env     string `json:"env,omitempty"`
	DryRun  bool   `json:"dry_run,omitempty"`
}

type batchRecordResult struct {
	Index    int                   `json:"index"`
	Handle   string                `json:"handle,omitempty"`
	OK       bool                  `json:"ok"`
	ID       string                `json:"id,omitempty"`
	Findings []onboardlint.Finding `json:"findings,omitempty"`
}

type onboardBatchResponse struct {
	DryRun  bool                `json:"dry_run"`
	Summary batchSummary        `json:"summary"`
	Results []batchRecordResult `json:"results"`
}

type batchSummary struct {
	Total    int `json:"total"`
	Created  int `json:"created"`
	Valid    int `json:"valid"`
	Rejected int `json:"rejected"`
}

func (s *Server) apiOnboardBatch(w http.ResponseWriter, r *http.Request) {
	var req onboardBatchRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "body must be JSON: {content, format?, env?, dry_run?}")
		return
	}
	if strings.TrimSpace(req.Content) == "" {
		onboardError(w, http.StatusBadRequest, "empty_content", "content is required")
		return
	}
	env := strings.TrimSpace(req.Env)
	if env != "" && !envPattern.MatchString(env) {
		onboardError(w, http.StatusBadRequest, "bad_env", "env must be a lowercase goatnet slug (e.g. galaxy)")
		return
	}
	records, err := parseBatch(req.Content, req.Format)
	if err != nil {
		onboardError(w, http.StatusBadRequest, "parse_error", err.Error())
		return
	}
	if len(records) == 0 {
		onboardError(w, http.StatusBadRequest, "empty_batch", "no manifest records found (expected a JSON array or YAML sequence)")
		return
	}

	sponsor := onboardAPIActor(r)
	resp := onboardBatchResponse{DryRun: req.DryRun}
	resp.Summary.Total = len(records)
	for i, rec := range records {
		stampAdded(rec)
		canonical, merr := json.Marshal(rec)
		if merr != nil {
			resp.Results = append(resp.Results, batchRecordResult{Index: i, OK: false,
				Findings: []onboardlint.Finding{{Severity: "error", Field: "(encode)", Message: merr.Error()}}})
			resp.Summary.Rejected++
			continue
		}
		handle, email := manifestHandleEmail(rec)
		res := onboardLint(canonical, "json")
		rr := batchRecordResult{Index: i, Handle: handle, OK: res.OK, Findings: res.Findings}
		if !res.OK {
			resp.Summary.Rejected++
			resp.Results = append(resp.Results, rr)
			continue
		}
		resp.Summary.Valid++
		if !req.DryRun {
			lintJSON, err := json.Marshal(res)
			_ = err // lint response is always JSON-marshalable; encode failure is not expected here.
			p := &onboardstore.Proposal{
				Sponsor: sponsor, Env: env, Handle: handle, Email: email,
				Status: onboardstore.ProposalProposed, ManifestJSON: string(canonical),
				LintJSON: string(lintJSON), Complete: true,
				// The manifest arrives whole from an operator, so affiliation.id
				// is the submitter's assertion rather than a candidate's typed
				// claim — no second decision for the review page to ask for.
				OrgSource: onboardstore.OrgSourceSponsor,
			}
			if cerr := s.Onboard.CreateProposal(r.Context(), p); cerr != nil {
				rr.OK = false
				rr.Findings = []onboardlint.Finding{{Severity: "error", Field: "(store)", Message: "could not store proposal"}}
				resp.Summary.Valid--
				resp.Summary.Rejected++
			} else {
				rr.ID = p.ID
				resp.Summary.Created++
			}
		}
		resp.Results = append(resp.Results, rr)
	}
	onboardJSON(w, http.StatusOK, resp)
}

// parseBatch decodes a JSON array or YAML sequence of manifest objects.
func parseBatch(content, format string) ([]map[string]any, error) {
	trimmed := strings.TrimSpace(content)
	useJSON := format == "json" || (format == "" && strings.HasPrefix(trimmed, "["))
	var list []map[string]any
	if useJSON {
		if err := json.Unmarshal([]byte(content), &list); err != nil {
			return nil, &batchParseError{"JSON array parse error: " + err.Error()}
		}
		return list, nil
	}
	var raw []map[string]any
	if err := yaml.Unmarshal([]byte(content), &raw); err != nil {
		return nil, &batchParseError{"YAML sequence parse error: " + err.Error()}
	}
	return raw, nil
}

type batchParseError struct{ msg string }

func (e *batchParseError) Error() string { return e.msg }
