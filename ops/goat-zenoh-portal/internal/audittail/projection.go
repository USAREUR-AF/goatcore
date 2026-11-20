package audittail

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
)

// MetadataObject never exposes unknown fields or body-bearing legacy records.
// Buffer the bounded projection before returning, so malformed final rows cannot
// turn into a successful partial history response.
func (p *Presigner) MetadataObject(ctx context.Context, key string) ([]byte, error) {
	if p.initErr != nil {
		return nil, p.initErr
	}
	source, _, err := ObjectSource(key, p.cfg.SubscriberID)
	if err != nil {
		return nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, strings.TrimRight(p.cfg.Endpoint, "/")+"/"+p.cfg.Bucket+"/"+s3EscapePath(key), nil)
	if err != nil {
		return nil, err
	}
	if err := signV4Request(p.cfg, req, nil); err != nil {
		return nil, err
	}
	resp, err := p.cli.Do(req)
	if err != nil {
		return nil, fmt.Errorf("read audit object failed")
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("read audit object: status %d", resp.StatusCode)
	}
	const maxObject = 256 << 20
	limited := &io.LimitedReader{R: resp.Body, N: maxObject + 1}
	scanner := bufio.NewScanner(limited)
	scanner.Buffer(make([]byte, 65536), 4<<20)
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	fields := strings.Fields("schema_version event_id anchor vendor identity_model key_expr ts src_zid source_provenance payload_hash payload_len payload_capture onboarding_classification audit_subscriber_id sample_kind receiving_site outcome namespace_slot namespace_attribution policy_digest policy_evidence")
	rows := 0
	for scanner.Scan() {
		if len(bytes.TrimSpace(scanner.Bytes())) == 0 {
			continue
		}
		var raw map[string]json.RawMessage
		if err := json.Unmarshal(scanner.Bytes(), &raw); err != nil || raw == nil {
			return nil, fmt.Errorf("invalid audit record")
		}
		var version int
		if v, ok := raw["schema_version"]; ok {
			if json.Unmarshal(v, &version) != nil {
				return nil, fmt.Errorf("invalid schema version")
			}
		}
		if (source == "legacy" && version > 1) || (source != "legacy" && version != 2) {
			return nil, fmt.Errorf("audit schema/filename mismatch")
		}
		if source != "legacy" {
			if _, ok := raw["payload_bytes_b64"]; ok {
				return nil, fmt.Errorf("body field in metadata object")
			}
		}
		clean := make(map[string]json.RawMessage)
		for _, name := range fields {
			if v, ok := raw[name]; ok {
				if err := validateMetadataField(name, v); err != nil {
					return nil, err
				}
				clean[name] = v
			}
		}
		// Legacy records cannot establish fields that their schema never defined.
		if source == "legacy" {
			for _, name := range strings.Fields("sample_kind receiving_site outcome namespace_attribution policy_evidence") {
				clean[name] = json.RawMessage(`"unknown"`)
			}
			clean["namespace_slot"] = json.RawMessage(`null`)
			clean["policy_digest"] = json.RawMessage(`null`)
			clean["source_provenance"] = json.RawMessage(`"unavailable"`)
		}
		if err := encoder.Encode(clean); err != nil {
			return nil, err
		}
		rows++
		if out.Len() > 64<<20 || rows > 100000 {
			return nil, fmt.Errorf("metadata projection exceeds bounds")
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("invalid or oversized audit record")
	}
	if limited.N <= 0 || rows == 0 {
		return nil, fmt.Errorf("empty or oversized audit object")
	}
	return out.Bytes(), nil
}

// Only bounded scalar metadata may cross the historical projection boundary.
func validateMetadataField(name string, value json.RawMessage) error {
	if name == "schema_version" || name == "payload_len" {
		var n uint64
		if string(value) == "null" || json.Unmarshal(value, &n) != nil {
			return fmt.Errorf("invalid numeric metadata field %s", name)
		}
		return nil
	}
	if (name == "namespace_slot" || name == "policy_digest") && string(value) == "null" {
		return nil
	}
	var s string
	if string(value) == "null" || json.Unmarshal(value, &s) != nil || len(s) > 16384 {
		return fmt.Errorf("invalid string metadata field %s", name)
	}
	return nil
}
