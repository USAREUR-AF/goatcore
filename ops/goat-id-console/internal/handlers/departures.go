package handlers

import (
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// departures.go — the goatnet's register of who has left (ADR 1272): every
// person whose access cut was queued, is running, finished or failed, with
// the reason, the day it took effect, who asked for it, and a link to the
// per-plane verification page. Read-only; the cut is the Offboard button on
// the onboarding page.

// departureRow is one line of the register.
type departureRow struct {
	Handle      string
	Env         string
	Status      string // offboard-requested | offboarding | offboarded | offboard-failed
	ReasonClass string // "" on a row cut before the console asked for a reason
	ReasonText  string
	Effective   string
	RequestedBy string
	RequestedAt int64
	// State is what the register claims about the row, as data: "cut" when
	// the runner finished, "open" while it is queued, running or failed.
	State string
	Note  string
}

// departuresVars is the page payload.
type departuresVars struct {
	Rows []departureRow
	Open int
	Err  string
}

func departureRows(ps []onboardstore.Proposal) ([]departureRow, int) {
	rows, open := make([]departureRow, 0, len(ps)), 0
	for _, p := range ps {
		row := departureRow{
			Handle: p.Handle, Env: p.Env, Status: p.Status,
			ReasonClass: p.OffboardReasonClass, ReasonText: p.OffboardReasonText,
			Effective: p.OffboardEffective, RequestedBy: p.OffboardRequestedBy,
			RequestedAt: p.OffboardRequestedAt, State: "open",
		}
		if p.Status == onboardstore.ProposalOffboarded {
			row.State = "cut"
		} else {
			open++
		}
		if p.Status == onboardstore.ProposalOffboardFailed {
			row.Note = p.Note
		}
		rows = append(rows, row)
	}
	return rows, open
}

// handleDepartures renders GET /departures.
func (s *Server) handleDepartures(w http.ResponseWriter, r *http.Request) {
	vars := &pageVars{Title: "Departures", Departures: &departuresVars{}}
	if s.Onboard == nil {
		vars.Departures.Err = "onboarding is not enabled on this console, so it holds no departures"
		s.render(w, r, "departures", vars)
		return
	}
	ps, err := s.Onboard.ListDepartures(r.Context())
	if err != nil {
		vars.Departures.Err = "the register is unreadable: " + err.Error()
		s.render(w, r, "departures", vars)
		return
	}
	vars.Departures.Rows, vars.Departures.Open = departureRows(ps)
	s.render(w, r, "departures", vars)
}
