// Package canon produces the canonical JSON bytes that goat carriage signatures cover.
//
// Canonical form (pod-carriage-and-provenance.md §7.1): object keys sorted, no insignificant
// whitespace, numbers kept exactly as written, '<', '>' and '&' not escaped, no trailing newline.
package canon

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
)

// FromJSON parses raw JSON and returns its canonical encoding.
func FromJSON(raw []byte) ([]byte, error) {
	v, err := Decode(raw)
	if err != nil {
		return nil, err
	}
	return Encode(v)
}

// Decode parses raw JSON into generic values, keeping numbers as json.Number.
func Decode(raw []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, fmt.Errorf("parse JSON: %w", err)
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return nil, errors.New("parse JSON: trailing data after the document")
	}
	return v, nil
}

// Encode writes generic values (maps, slices, strings, json.Number, bool, nil) canonically.
// encoding/json sorts map keys; HTML escaping is turned off.
func Encode(v any) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, fmt.Errorf("encode JSON: %w", err)
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

// Marshal encodes any Go value canonically by round-tripping it through generic values.
func Marshal(v any) ([]byte, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, fmt.Errorf("encode JSON: %w", err)
	}
	return FromJSON(raw)
}

// WithoutKey returns the canonical encoding of the top-level object in raw with key removed,
// and the string value that key held ("" when absent or not a string).
func WithoutKey(raw []byte, key string) (body []byte, value string, err error) {
	v, err := Decode(raw)
	if err != nil {
		return nil, "", err
	}
	obj, ok := v.(map[string]any)
	if !ok {
		return nil, "", errors.New("document is not a JSON object")
	}
	if s, ok := obj[key].(string); ok {
		value = s
	}
	delete(obj, key)
	body, err = Encode(obj)
	return body, value, err
}
