package api

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// mintSlotLeaf mints the fabric leaf for slot anchor at generation gen and
// records it in slot_leaves (GHSA-rr3p). The leaf's CN is
// registry.LeafCN(anchor, gen); everything else about the mint (namespace,
// URI-SAN, tarball, recipient-cert file) stays keyed on the bare anchor.
//
// The caller has already moved the slot to gen: the assignment did, or
// BumpSlotGeneration did for a rotation or a collection. A leaf that cannot be
// recorded, or whose generation the slot has already left, is an error and the
// caller must not hand it out. The record is the reuse quarantine; a leaf
// without one would let the slot be reissued while the leaf is valid.
//
// identityID is the identity the caller minted for ("" when it has none, as
// for a machine); the record is refused if the slot is linked to someone else
// by the time the leaf is recorded.
func (s *Server) mintSlotLeaf(ctx context.Context, anchor, apiKey string, gen int64, serverAuth bool, identityID string) (*certmint.Result, error) {
	cn := registry.LeafCN(anchor, gen)
	if a, g, ok := registry.AnchorOfCN(cn); !ok || a != anchor || g != gen {
		return nil, fmt.Errorf("refusing to mint %q: not a leaf CN of slot %s", cn, anchor)
	}
	res, err := s.minter.Mint(ctx, anchor, apiKey, certmint.MintOptions{
		CommonName: cn,
		ServerAuth: serverAuth,
	})
	if err != nil {
		return nil, err
	}
	if err := s.reg.RecordSlotLeaf(ctx, registry.SlotLeaf{
		SlotID:            anchor,
		IssuedForIdentity: identityID,
		Generation:        gen,
		Serial:            res.Serial,
		NotBefore:         res.NotBefore.Unix(),
		NotAfter:          res.NotAfter.Unix(),
	}); err != nil {
		return nil, fmt.Errorf("record leaf: %w", err)
	}
	return res, nil
}

// Leaf status values (leaf_status in responses).
const (
	leafLive    = "live"    // the router has loaded the fragment naming this CN
	leafPending = "pending" // the router has not loaded it yet; it will at its next reload
	leafUnknown = "unknown" // no record of what the router loaded on this host
)

// leafStatus says whether the router has loaded the fragment that names the
// slot's leaf CN at gen, from the record the router host's reload watcher
// writes beside the config (the portal has no docker socket and never
// restarts the router).
func (s *Server) leafStatus(anchor string, gen int64) string {
	if s.aclRender == nil {
		return leafUnknown
	}
	cns, ok := s.aclRender.LoadedLeafCNs()
	if !ok {
		return leafUnknown
	}
	if cns[registry.LeafCN(anchor, gen)] {
		return leafLive
	}
	return leafPending
}

// revocationStatus says whether the router has stopped admitting a revoked
// leaf CN: "live" (the router's loaded slot subjects no longer name it),
// "pending_router_reload" (they still do), "unknown" (no record of what the
// router loaded on this host) or "none" (no assigned leaf was revoked).
func (s *Server) revocationStatus(revokedCN string) (string, string) {
	if revokedCN == "" {
		return "none", "No assigned slot was freed, so no fabric leaf was revoked."
	}
	var cns map[string]bool
	ok := false
	if s.aclRender != nil {
		cns, ok = s.aclRender.LoadedLeafCNs()
	}
	switch {
	case !ok:
		return "unknown", "The slot is freed in the registry and the router's slot subjects are re-rendered without this leaf, " +
			"but this host keeps no record of what the router has loaded: the router admits the leaf until it is restarted. " +
			"The revocation is not complete on the fabric until then."
	case cns[revokedCN]:
		return "pending_router_reload", "The slot is freed and the router's slot subjects are re-rendered without this leaf, " +
			"but the router still admits it until its reload watcher recreates it (normally within about a minute). " +
			"The revocation is not complete on the fabric until fabric_revocation reads live."
	default:
		return "live", "The router no longer admits this leaf."
	}
}

// leafStatusNote is the sentence a response carries about when the router
// admits the leaf it hands out.
func leafStatusNote(status string) string {
	switch status {
	case leafLive:
		return "The router already admits this leaf."
	case leafPending:
		return "The router admits this leaf at its next reload of the slot subjects, within its reload interval (5 minutes by default); until then a connection with it is refused."
	default:
		return "The router admits this leaf once it reloads the slot subjects: within the reload watcher's interval (5 minutes by default), or at the next router restart where no watcher runs."
	}
}

// syncSlotFragment re-renders the router's slot fragment from the registry
// after a slot changed hands or generation. Per-uuid-slot-pool mode only. It
// never fails the request: the registry write already happened, and the next
// sync (any later slot change, or a portal restart) renders the same state.
// What it cannot do is make the router load it; on zenoh 1.9 that is a router
// restart (F-156), so a revoke or rotate takes effect on the wire at the next
// restart.
func (s *Server) syncSlotFragment(ctx context.Context) {
	if s.cfg.IdentityModel != config.IdentityModelPerUUID || s.aclRender == nil {
		return
	}
	if _, err := SyncSlotFragment(context.WithoutCancel(ctx), s.reg, s.aclRender, s.cfg); err != nil {
		slog.Warn("slot fragment: render failed; the router keeps its previous slot subjects until the next successful render",
			"err", err)
	}
}

// slotFragmentSyncMu serializes every slot-fragment render in this process
// (requests, the boot render, the periodic re-render).
var slotFragmentSyncMu sync.Mutex

// SyncSlotFragment renders the slot fragment for every pool slot and applies
// it through r. Exported for the portal's boot-time render. cfg supplies the
// data roster the carriage rules resolve org membership against (ADR 1251
// phase 4: the carry-<anchor> subjects); nil renders no carriers.
//
// The registry read and the write happen under one lock (GHSA-rr3p review r8
// F4): without it, a sync that read its bindings before an offboard or grant
// revoke committed could land after that change's own sync and restore the
// old CN.
func SyncSlotFragment(ctx context.Context, reg *registry.Registry, r *acl.Renderer, cfg *config.Config) (acl.SlotFragmentResult, error) {
	slotFragmentSyncMu.Lock()
	defer slotFragmentSyncMu.Unlock()
	res, err := r.ApplySlotFragmentFrom(ctx, func(ctx context.Context) ([]acl.SlotBinding, error) {
		bs, err := reg.SlotBindings(ctx)
		if err != nil {
			return nil, err
		}
		var carriers map[string][]string
		if cfg != nil {
			sn, err := loadCarriageSnapshot(ctx, reg, cfg, &fragmentRosters)
			if err != nil {
				return nil, fmt.Errorf("carriage: %w", err)
			}
			carriers = sn.carriers()
		}
		in := make([]acl.SlotBinding, 0, len(bs))
		for _, b := range bs {
			ab := acl.SlotBinding{Anchor: b.Anchor, CN: b.CN, Carry: carriers[b.Anchor]}
			for _, c := range b.Copub {
				ab.Copub = append(ab.Copub, acl.CopubSubject{Key: c.Key, Live: c.Live})
			}
			in = append(in, ab)
		}
		return in, nil
	})
	if errors.Is(err, acl.ErrNoSlotMarkers) {
		return res, fmt.Errorf("%w: the fragment file was written, and the router role's next render includes it", err)
	}
	return res, err
}
