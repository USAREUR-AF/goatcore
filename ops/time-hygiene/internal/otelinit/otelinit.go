// Package otelinit centralizes the OpenTelemetry trace-provider boot sequence
// for ops/time-hygiene's binaries (today ntp-health-agent) under Block 27C /
// Block 50B.4.
//
// Every binary in this Go module boots tracing the same way, so the sequence
// lives here once rather than in each cmd/ — the same shape as the shared
// internal/safego package.
//
// Per-service span attributes + sampling shapes stay in the consuming cmd's
// otel.go; this package owns only:
//
//   - reading OTEL_EXPORTER_OTLP_ENDPOINT
//   - constructing the OTLP/HTTP exporter
//   - building the OTel resource with the right service.* + dbb.site_role attrs
//   - registering the global TracerProvider
//
// Anchors:
//
//	docs/adr/0822-wide-event-observability-otlp-openobserve.md
//	docs/design/observability-wide-events.md  (§ "OTLP attribute mapping")
//	docs/project/block-27c-scoping-2026-05-05.md
//
// Telemetry is best-effort: when OTEL_EXPORTER_OTLP_ENDPOINT is unset, Init
// returns a no-op shutdown and leaves the default no-op tracer provider in
// place. Span-emitting code paths run uniformly regardless — "OTel disabled"
// is structurally indistinguishable from "OTel enabled" at the call sites.
package otelinit

import (
	"context"
	"fmt"
	"net/url"
	"os"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	semconv "go.opentelemetry.io/otel/semconv/v1.27.0"
)

// Resource attribute key for the per-service site role (mgmt, obs, etc.).
// Matches mgmt-host-bootstrap's convention so OpenObserve queries join cleanly
// across producers.
const AttrSiteRole = "dbb.site_role"

// Opts parameterize the producer's resource attributes. ServiceName +
// ServiceVersion are required; Extra is appended after the canonical set so
// callers can attach service-specific resource-level signals (e.g. listener
// addr, daemon mode) without forking buildResource.
type Opts struct {
	// ServiceName is the OTel `service.name`. Hardcoded per binary — this
	// becomes the dominant filter dimension in OpenObserve, so don't pull it
	// from env.
	ServiceName string

	// ServiceVersion is the OTel `service.version`. Use the consuming cmd's
	// `version` const so a release-tag bump is visible in telemetry.
	ServiceVersion string

	// Extra are appended to the resource as additional attributes. Use for
	// per-service flavor data that's stable across an entire process lifetime
	// (listener addr, peer LRU cap, etc.) — never per-request data.
	Extra []attribute.KeyValue
}

// Init configures the global OpenTelemetry trace provider from env per Opts
// and returns a shutdown function. The caller is responsible for bounding the
// shutdown context (typically 5s) so a slow / unreachable endpoint cannot
// stall process exit.
//
// Resource attributes set:
//
//	service.name        = opts.ServiceName
//	service.version     = opts.ServiceVersion
//	service.namespace   = $DBB_SITE_ID  (default "unknown")
//	service.instance.id = os.Hostname() (default "unknown")
//	dbb.site_role       = $DBB_SITE_ROLE (default "mgmt")
//	... + opts.Extra
//
// When OTEL_EXPORTER_OTLP_ENDPOINT is unset, returns a no-op shutdown and
// leaves the default no-op tracer provider in place.
func Init(ctx context.Context, opts Opts) (func(context.Context) error, error) {
	if opts.ServiceName == "" {
		return nil, fmt.Errorf("otelinit: ServiceName required")
	}

	endpoint := os.Getenv("OTEL_EXPORTER_OTLP_ENDPOINT")
	if endpoint == "" {
		return func(context.Context) error { return nil }, nil
	}

	clientOpts, err := HTTPOptionsFromEndpoint(endpoint)
	if err != nil {
		return nil, fmt.Errorf("parse OTEL_EXPORTER_OTLP_ENDPOINT: %w", err)
	}

	exporter, err := otlptrace.New(ctx, otlptracehttp.NewClient(clientOpts...))
	if err != nil {
		return nil, fmt.Errorf("create OTLP exporter: %w", err)
	}

	tp := sdktrace.NewTracerProvider(
		sdktrace.WithBatcher(exporter),
		sdktrace.WithResource(BuildResource(opts)),
	)
	otel.SetTracerProvider(tp)
	return tp.Shutdown, nil
}

// HTTPOptionsFromEndpoint converts an OTEL_EXPORTER_OTLP_ENDPOINT URL into
// otlptracehttp client options. Recognizes http/https schemes and preserves
// the URL path (defaults to /v1/traces if empty per OTLP spec). Exported so
// otel_test.go in each cmd can exercise the parse path without spinning the
// SDK.
func HTTPOptionsFromEndpoint(endpoint string) ([]otlptracehttp.Option, error) {
	u, err := url.Parse(endpoint)
	if err != nil {
		return nil, err
	}
	if u.Host == "" {
		return nil, fmt.Errorf("endpoint %q missing host", endpoint)
	}
	opts := []otlptracehttp.Option{otlptracehttp.WithEndpoint(u.Host)}
	if u.Scheme == "http" {
		opts = append(opts, otlptracehttp.WithInsecure())
	}
	if u.Path != "" && u.Path != "/" {
		opts = append(opts, otlptracehttp.WithURLPath(u.Path))
	}
	return opts, nil
}

// BuildResource assembles the OTel resource describing this process. Exported
// so otel_test.go can assert resource attributes without spinning the SDK.
func BuildResource(opts Opts) *resource.Resource {
	siteID := envOrDefault("DBB_SITE_ID", "unknown")
	siteRole := envOrDefault("DBB_SITE_ROLE", "mgmt")
	hostname, err := os.Hostname()
	if err != nil || hostname == "" {
		hostname = "unknown"
	}
	attrs := make([]attribute.KeyValue, 0, 5+len(opts.Extra))
	attrs = append(attrs,
		semconv.ServiceName(opts.ServiceName),
		semconv.ServiceVersion(opts.ServiceVersion),
		semconv.ServiceNamespace(siteID),
		semconv.ServiceInstanceID(hostname),
		attribute.String(AttrSiteRole, siteRole),
	)
	attrs = append(attrs, opts.Extra...)
	return resource.NewWithAttributes(semconv.SchemaURL, attrs...)
}

func envOrDefault(name, def string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return def
}
