package handlers

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"strings"
	"time"
)

const meshMoveConfirmationTTL = 5 * time.Minute

type meshMoveBinding struct {
	ActorID    string `json:"actor_id"`
	ActorEmail string `json:"actor_email"`
	Realm      string `json:"realm"`
	Identity   string `json:"identity"`
	Site       string `json:"site"`
	Peer       string `json:"peer"`
	From       string `json:"from"`
	To         string `json:"to"`
}

type meshMoveConfirmation struct {
	Authority string            `json:"authority"`
	Nonce     string            `json:"nonce"`
	Expires   int64             `json:"expires"`
	Binding   meshMoveBinding   `json:"binding"`
	PlanHash  [sha256.Size]byte `json:"plan_hash"`
}

func meshMovePlanHash(plan meshMovePlan) ([sha256.Size]byte, error) {
	plan.ConfirmationToken = ""
	plan.ConfirmationState = ""
	plan.Declaration = nil
	payload, err := json.Marshal(plan)
	return sha256.Sum256(payload), err
}

func (s *Server) issueMeshMoveConfirmation(binding meshMoveBinding, plan meshMovePlan, now time.Time) string {
	if len(plan.Movers) != 1 || plan.Authority == authorityUnknown || (binding.ActorID == "" && binding.ActorEmail == "") {
		return ""
	}
	hash, err := meshMovePlanHash(plan)
	if err != nil {
		return ""
	}
	payload, err := json.Marshal(meshMoveConfirmation{
		Authority: plan.Authority, Nonce: mintNonce(), Expires: now.Add(meshMoveConfirmationTTL).Unix(), Binding: binding, PlanHash: hash,
	})
	if err != nil {
		return ""
	}
	mac := hmac.New(sha256.New, s.meshMoveConfirmationKey[:])
	_, _ = mac.Write(payload)
	return base64.RawURLEncoding.EncodeToString(payload) + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

func (s *Server) readMeshMoveConfirmation(token string, binding meshMoveBinding, now time.Time) (meshMoveConfirmation, bool) {
	var confirmation meshMoveConfirmation
	if len(token) > 8192 {
		return confirmation, false
	}
	data, signature, ok := strings.Cut(token, ".")
	if !ok {
		return confirmation, false
	}
	payload, payloadErr := base64.RawURLEncoding.DecodeString(data)
	sig, sigErr := base64.RawURLEncoding.DecodeString(signature)
	if payloadErr != nil || sigErr != nil {
		return confirmation, false
	}
	mac := hmac.New(sha256.New, s.meshMoveConfirmationKey[:])
	_, _ = mac.Write(payload)
	if !hmac.Equal(sig, mac.Sum(nil)) || json.Unmarshal(payload, &confirmation) != nil {
		return meshMoveConfirmation{}, false
	}
	return confirmation, confirmation.Binding == binding && now.Unix() < confirmation.Expires
}
