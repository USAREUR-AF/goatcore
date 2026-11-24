// goat-pod-api is the pod-side carriage runtime (ADR 1251, pod-carriage-and-provenance.md):
// it pulls and verifies the pod's signed rule generation, renders the pod router's ACL with any
// local rules, applies it (a failed addition rolls back, a failed removal fails closed), keeps the signed local-rule ledger, and
// serves the local API and the Gatepost page.
//
//	goat-pod-api serve  --config <file>   run the service
//	goat-pod-api render --config <file> --out <file>
//	                                      render the router configuration from the saved state
//	                                      (first-boot uses this; nothing is restarted)
//	goat-pod-api adopt  --config <file> <generation.json>
//	                                      verify and apply a generation file (for a pod with no
//	                                      portal link, or a test)
//	goat-pod-api status --config <file>   print the status JSON from the saved state
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/router"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/server"
)

const usage = `usage: goat-pod-api <serve|render|adopt|status> --config <file> [options]`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, usage)
		os.Exit(2)
	}
	if err := run(os.Args[1], os.Args[2:]); err != nil {
		fmt.Fprintln(os.Stderr, "goat-pod-api:", err)
		os.Exit(1)
	}
}

func run(cmd string, args []string) error {
	fs := flag.NewFlagSet(cmd, flag.ContinueOnError)
	cfgPath := fs.String("config", "/etc/goat-pod-api/config.json", "configuration file")
	out := fs.String("out", "", "render: output file")
	if err := fs.Parse(args); err != nil {
		return err
	}
	cfg, err := config.Load(*cfgPath)
	if err != nil {
		return err
	}
	a, err := agent.New(cfg, router.Docker{Container: cfg.Router.Container})
	if err != nil {
		return err
	}
	switch cmd {
	case "serve":
		return serve(cfg, a)
	case "render":
		if *out == "" {
			return errors.New("render needs --out")
		}
		rendered, _, err := a.Render()
		if err != nil {
			return err
		}
		return os.WriteFile(*out, rendered, 0o600) // #nosec G703 -- the operator names the output file
	case "adopt":
		if fs.NArg() != 1 {
			return errors.New("adopt needs one generation file")
		}
		raw, err := os.ReadFile(fs.Arg(0)) // #nosec G703 -- the operator names the generation file
		if err != nil {
			return err
		}
		return a.Adopt(context.Background(), raw)
	case "status":
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(a.Status())
	default:
		return fmt.Errorf("unknown command %q\n%s", cmd, usage)
	}
}

func serve(cfg *config.Config, a *agent.Agent) error {
	if len(cfg.Listen) == 0 {
		return errors.New("listen names no address")
	}
	tc, err := server.TLSConfig(cfg.PodCert, cfg.PodKey, cfg.CARoots)
	if err != nil {
		return err
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	h := (&server.Server{Agent: a, GatepostPage: cfg.GatepostPage}).Handler()
	errc := make(chan error, len(cfg.Listen))
	var servers []*http.Server
	for _, addr := range cfg.Listen {
		srv := server.NewHTTPServer(addr, h, tc)
		servers = append(servers, srv)
		go func() {
			a.Log.Printf("listening on https://%s", srv.Addr)
			if err := srv.ListenAndServeTLS("", ""); err != nil && !errors.Is(err, http.ErrServerClosed) {
				errc <- fmt.Errorf("listen %s: %w", srv.Addr, err)
			}
		}()
	}
	go a.Run(ctx)
	select {
	case <-ctx.Done():
	case err = <-errc:
	}
	shutdown, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	for _, srv := range servers {
		_ = srv.Shutdown(shutdown)
	}
	return err
}
