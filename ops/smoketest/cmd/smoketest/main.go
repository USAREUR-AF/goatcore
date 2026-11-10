// Command smoketest runs probes against a goatnet's infrastructure
// or starts an HTTP server that exposes probe results via a dashboard.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/usareur-af/goatcore/ops/smoketest/internal/catalogue"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/netbird"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/probe"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/server"
	"github.com/usareur-af/goatcore/ops/smoketest/internal/store"
)

func main() {
	log.SetFlags(log.LstdFlags | log.Lshortfile)

	if len(os.Args) < 2 {
		usage()
		os.Exit(1)
	}

	switch os.Args[1] {
	case "run":
		cmdRun(os.Args[2:])
	case "serve":
		cmdServe(os.Args[2:])
	case "help", "-h", "--help":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", os.Args[1])
		usage()
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprintln(os.Stderr, `Usage: smoketest <command> [flags]

Commands:
  run     Run smoke test probes and print results
  serve   Start the HTTP dashboard server

Run flags:
  --mode       quick|full (default: quick)
  --category   Comma-separated categories to run (default: all)
  --json       Output results as JSON
  --config     Goatnet config YAML: domain, relays, host_mesh_ip, relay_dns, expected_ips

Serve flags:
  --addr           Listen address (default: :8080)
  --results-file   Path to JSONL results file (default: results.jsonl)
  --catalogue      Path to service catalogue JSON file
  --config         Goatnet config YAML (as for run)

Environment:
  NETBIRD_API_TOKEN   Netbird management API token
  NETBIRD_DOMAIN      Netbird management domain (overrides the config file's domain)
  SMOKETEST_CONFIG    Goatnet config YAML when --config is not given`)
}

func cmdRun(args []string) {
	fs := flag.NewFlagSet("run", flag.ExitOnError)
	mode := fs.String("mode", "quick", "run mode: quick or full")
	category := fs.String("category", "", "comma-separated categories to run (empty = all)")
	jsonOut := fs.Bool("json", false, "output results as JSON")
	cataloguePath := fs.String("catalogue", "", "path to service catalogue YAML file")
	configPath := fs.String("config", os.Getenv("SMOKETEST_CONFIG"), "goatnet config YAML (default $SMOKETEST_CONFIG)")
	fs.Parse(args)

	cfg, err := probe.LoadConfig(*configPath)
	if err != nil {
		log.Fatalf("load config: %v", err)
	}
	cfg.Mode = *mode
	cfg.NetbirdToken = os.Getenv("NETBIRD_API_TOKEN")

	if *cataloguePath != "" {
		cat, err := catalogue.Load(*cataloguePath)
		if err != nil {
			log.Fatalf("load catalogue: %v", err)
		}
		probe.SetServiceChecks(cat.ToServiceChecks())
	}

	if domain := os.Getenv("NETBIRD_DOMAIN"); domain != "" {
		cfg.Domain = domain
	}

	if *category != "" {
		for _, c := range strings.Split(*category, ",") {
			c = strings.TrimSpace(c)
			if c != "" {
				cfg.Categories = append(cfg.Categories, probe.Category(c))
			}
		}
	}

	runner := probe.NewRunner(cfg)
	result := runner.Run(context.Background())

	if *jsonOut {
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		if err := enc.Encode(result); err != nil {
			log.Fatalf("json encode: %v", err)
		}
	} else {
		probe.PrintTerminal(result)
	}

	if result.Fail > 0 {
		os.Exit(1)
	}
}

func cmdServe(args []string) {
	fs := flag.NewFlagSet("serve", flag.ExitOnError)
	addr := fs.String("addr", ":8080", "listen address")
	resultsFile := fs.String("results-file", "results.jsonl", "path to JSONL results file")
	cataloguePath := fs.String("catalogue", "", "path to service catalogue YAML file")
	dashboardDir := fs.String("dashboard-dir", "", "path to dashboard dist/ directory (uses embedded HTML if empty)")
	configPath := fs.String("config", os.Getenv("SMOKETEST_CONFIG"), "goatnet config YAML (default $SMOKETEST_CONFIG)")
	fs.Parse(args)

	cfg, err := probe.LoadConfig(*configPath)
	if err != nil {
		log.Fatalf("load config: %v", err)
	}
	cfg.NetbirdToken = os.Getenv("NETBIRD_API_TOKEN")

	if domain := os.Getenv("NETBIRD_DOMAIN"); domain != "" {
		cfg.Domain = domain
	}

	st := store.New(*resultsFile)

	var nb *netbird.Client
	if cfg.NetbirdToken != "" {
		nb = netbird.NewClient(cfg.Domain, cfg.NetbirdToken)
	}

	var cat *catalogue.File
	if *cataloguePath != "" {
		var err error
		cat, err = catalogue.Load(*cataloguePath)
		if err != nil {
			log.Fatalf("load catalogue: %v", err)
		}
		log.Printf("loaded %d services from catalogue", len(cat.Services))
	}

	// Wire catalogue service checks into the service probe.
	var svcEntries []server.ServiceEntry
	if cat != nil {
		probe.SetServiceChecks(cat.ToServiceChecks())
		for _, s := range cat.Services {
			// Extract the first HTTP/HTTPS URL from checks for display.
			var url string
			for _, c := range s.Checks {
				if c.URL != "" {
					url = c.URL
					break
				}
			}
			svcEntries = append(svcEntries, server.ServiceEntry{
				Name:        s.Name,
				Description: s.Description,
				URL:         url,
				Category:    s.Category,
			})
		}
	}

	srv := server.New(server.ServerConfig{
		ProbeCfg:     cfg,
		Store:        st,
		Netbird:      nb,
		Catalogue:    svcEntries,
		DashboardDir: *dashboardDir,
	})

	if err := srv.ListenAndServe(*addr); err != nil {
		log.Fatalf("server: %v", err)
	}
}
