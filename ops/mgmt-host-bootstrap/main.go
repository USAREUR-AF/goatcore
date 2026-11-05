// mgmt-host-bootstrap — sovereign netbird account + setup-key creation.
//
// Per ADR 0410 (Go CLI for sovereign peer enrollment) + ADR 0413 (no ORM
// in our own Go code). Replaces the broken DB-direct seed pattern that
// burned the cgnat-validation env — see the attempt-1
// findings doc §1.3 (corrected).
//
// Design: docs/design/identity-architecture.md §4.2.
//
// Pinned for compatibility with netbird-mgmt 0.68.3. CI tests live at
// ops/mgmt-host-bootstrap/cmd_test.go and run against a local
// docker-compose with the same image digest.
package main

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/jmoiron/sqlx"
	_ "github.com/jackc/pgx/v5/stdlib"
	"github.com/spf13/cobra"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/trace"
)

const version = "0.1.0"

// =============================================================================
// idempotency (Block 50B.8)
// =============================================================================
//
// Every write path runs through runIdempotent so a re-run with the same
// inputs is a no-op + exit 0 with stdout `already-applied: <key>`. Survives
// mid-bootstrap kills + retries from Ansible, which is the realistic failure
// mode for a one-shot CLI invoked by a higher-level orchestrator.
//
// Mechanism:
//
//   1. Each write command computes a deterministic key from its inputs
//      (e.g. sha256("create-account|owner|site-name|network|domain")) OR
//      accepts one explicitly via --idempotency-key.
//   2. runIdempotent opens a tx, takes a Postgres advisory lock keyed on
//      a 63-bit hash of the key (serializes concurrent re-runs of the same
//      command), checks bootstrap_idempotency for a matching row, and
//      either returns "already-applied" or runs fn + records the key on
//      commit.
//   3. The advisory lock auto-releases on tx commit / rollback, so kill -9
//      mid-run leaves no stuck lock.
//
// The bootstrap_idempotency table is created on first use; no separate
// migration tool. This is a one-table append-only ledger; if a future
// schema change is needed, we will fold it into a Block 50C
// codegen-from-SQL effort, not introduce a migrations framework today.

const idempotencyTableDDL = `
CREATE TABLE IF NOT EXISTS bootstrap_idempotency (
  key        text         PRIMARY KEY,
  command    text         NOT NULL,
  result     text         NOT NULL,
  created_at timestamptz  NOT NULL DEFAULT now()
)`

// errAlreadyApplied is returned by runIdempotent when the key is already
// recorded; the caller emits the recorded result and exits cleanly.
var errAlreadyApplied = errors.New("already-applied")

// ensureIdempotencyTable creates bootstrap_idempotency on first use. Safe
// to call repeatedly; CREATE TABLE IF NOT EXISTS is the standard idiom.
func ensureIdempotencyTable(ctx context.Context, db *sqlx.DB) error {
	_, err := db.ExecContext(ctx, idempotencyTableDDL)
	return err
}

// advisoryLockKey hashes an idempotency key into the 63-bit signed integer
// pg_advisory_xact_lock expects (single-arg form). Collisions are
// acceptable here — the lock just serializes execution, the actual
// duplicate-detection happens via the PRIMARY KEY check.
func advisoryLockKey(key string) int64 {
	sum := sha256.Sum256([]byte(key))
	u := binary.BigEndian.Uint64(sum[:8])
	// Mask off the sign bit so we always get a non-negative int64; the
	// advisory-lock space is signed, but we don't need the full range.
	return int64(u >> 1)
}

// runIdempotent wraps fn in the idempotency-key + advisory-lock pattern.
// Returns errAlreadyApplied (with the recorded result populated into
// result) if the key is already present; otherwise runs fn inside a tx,
// records the key, commits.
//
// fn receives the open tx; commit/rollback is owned by runIdempotent.
//
// The result string is what fn populated (typical: a short "id: ..." line)
// AND what the caller prints to stdout on the no-op replay path.
func runIdempotent(
	ctx context.Context,
	db *sqlx.DB,
	command, key string,
	fn func(tx *sqlx.Tx) (result string, err error),
) (result string, err error) {
	if err := ensureIdempotencyTable(ctx, db); err != nil {
		return "", fmt.Errorf("ensure idempotency table: %w", err)
	}

	tx, err := db.BeginTxx(ctx, nil)
	if err != nil {
		return "", fmt.Errorf("begin tx: %w", err)
	}
	defer func() {
		// Best-effort rollback; if Commit already ran this is a benign
		// "tx already committed" no-op.
		_ = tx.Rollback()
	}()

	// Serialize concurrent runs of the same key. Auto-released on tx end.
	if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock($1)`, advisoryLockKey(key)); err != nil {
		return "", fmt.Errorf("acquire advisory lock: %w", err)
	}

	// Already applied? Return the prior result.
	var prior string
	err = tx.GetContext(ctx, &prior,
		`SELECT result FROM bootstrap_idempotency WHERE key=$1`, key)
	if err == nil {
		// Commit the empty tx so the advisory lock releases promptly.
		if err := tx.Commit(); err != nil {
			return "", fmt.Errorf("commit idempotent replay: %w", err)
		}
		return prior, errAlreadyApplied
	}
	// Any error other than no-rows is a real failure.
	if !isNoRows(err) {
		return "", fmt.Errorf("check idempotency: %w", err)
	}

	// Not yet applied — run the work and record.
	result, err = fn(tx)
	if err != nil {
		return "", err
	}

	if _, err := tx.ExecContext(ctx,
		`INSERT INTO bootstrap_idempotency (key, command, result) VALUES ($1, $2, $3)`,
		key, command, result); err != nil {
		return "", fmt.Errorf("record idempotency: %w", err)
	}

	if err := tx.Commit(); err != nil {
		return "", fmt.Errorf("commit: %w", err)
	}
	return result, nil
}

// isNoRows reports whether err is the "no rows returned" sentinel from
// database/sql. We check string-equality to stay decoupled from the
// database/sql import path here.
func isNoRows(err error) bool {
	return err != nil && err.Error() == "sql: no rows in result set"
}

// idempotencyKeyFor returns sha256-hex of (command || NUL-separated inputs)
// suitable as a deterministic key when --idempotency-key is not provided.
func idempotencyKeyFor(command string, inputs ...string) string {
	h := sha256.New()
	h.Write([]byte(command))
	for _, in := range inputs {
		h.Write([]byte{0})
		h.Write([]byte(in))
	}
	return fmt.Sprintf("%s:%x", command, h.Sum(nil))
}

// resolveIdempotencyKey picks the explicit --idempotency-key flag if set,
// otherwise the auto-derived key from inputs.
func resolveIdempotencyKey(explicit, command string, inputs ...string) string {
	if explicit != "" {
		return explicit
	}
	return idempotencyKeyFor(command, inputs...)
}

// emitAlreadyApplied prints the canonical no-op message and the prior
// recorded result. Caller returns nil error after this.
func emitAlreadyApplied(key, prior string) {
	fmt.Printf("already-applied: %s\n", key)
	if prior != "" {
		fmt.Print(prior)
		if !strings.HasSuffix(prior, "\n") {
			fmt.Println()
		}
	}
}

func main() {
	// Best-effort OTel init. Failure logs to stderr and continues — the CLI
	// must never block on telemetry. Shutdown is bounded to 5s on exit so a
	// slow / unreachable endpoint cannot stall the operator.
	ctx := context.Background()
	shutdown, err := initOTel(ctx)
	if err != nil {
		fmt.Fprintln(os.Stderr, "otel init:", err)
		shutdown = func(context.Context) error { return nil }
	}
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := shutdown(ctx); err != nil {
			fmt.Fprintln(os.Stderr, "otel shutdown:", err)
		}
	}()

	root := &cobra.Command{
		Use:     "mgmt-host-bootstrap",
		Short:   "Sovereign netbird account + setup-key creation",
		Version: version,
	}

	// Default is EMPTY, not os.Getenv("PGDSN"): cobra prints every flag's
	// resolved default in --help, so an env-sourced default leaks the live
	// password into help text — which site-bootstrap's wrapper smoke task
	// then republishes into 0644 play logs.
	// $PGDSN is consumed in connect() instead; behavior is unchanged.
	dsn := root.PersistentFlags().String("dsn", "",
		"Postgres DSN (default $PGDSN). Example: host=postgres-a user=postgres password=... dbname=netbird sslmode=disable client_encoding=UTF8")

	root.AddCommand(
		cmdCreateAccount(dsn),
		cmdCreateSetupKey(dsn),
		cmdRotateSetupKey(dsn),
		cmdRevokePeer(dsn),
		cmdListPeers(dsn),
		cmdListSetupKeys(dsn),
	)

	if err := root.ExecuteContext(ctx); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// connect opens a sqlx connection and pings. An empty dsn falls back to
// $PGDSN here, NOT at flag registration — see the --dsn comment (help-text
// secret leak).
func connect(dsn string) (*sqlx.DB, error) {
	if dsn == "" {
		dsn = os.Getenv("PGDSN")
	}
	if dsn == "" {
		return nil, fmt.Errorf("--dsn or $PGDSN required")
	}
	db, err := sqlx.Connect("pgx", dsn)
	if err != nil {
		return nil, fmt.Errorf("connect: %w", err)
	}
	return db, nil
}

// hashSetupKey replicates netbird's GenerateSetupKey hash computation:
//
//	hashedKey := sha256.Sum256([]byte(strings.ToUpper(uuid)))
//	encodedHashedKey := base64.StdEncoding.EncodeToString(hashedKey[:])
//
// Source: netbird/management/server/types/setupkey.go:155-187.
// The plaintext stored/presented is the UPPERCASE UUID.
func hashSetupKey(plaintextUUID string) string {
	upper := strings.ToUpper(plaintextUUID)
	h := sha256.Sum256([]byte(upper))
	return base64.StdEncoding.EncodeToString(h[:])
}

// hiddenKey replicates netbird's HiddenKey(key, length) for key_secret.
// Source: netbird/management/server/types/setupkey.go:111-117.
func hiddenKey(plaintextUUID string) string {
	upper := strings.ToUpper(plaintextUUID)
	if len(upper) < 5 {
		return upper + "****"
	}
	return upper[:5] + "****"
}

// networkNetJSON returns the JSON-marshaled net.IPNet form netbird
// expects in accounts.network_net (gorm:"serializer:json" on net.IPNet).
// Format: {"IP":"100.64.0.0","Mask":"//8AAA=="} — IP as quoted string,
// Mask as base64 of the 4-byte mask. Verified against
// netbird-mgmt 0.68.3 in local docker-compose .
func networkNetJSON(cidr string) (string, error) {
	_, ipnet, err := net.ParseCIDR(cidr)
	if err != nil {
		return "", fmt.Errorf("parse CIDR %q: %w", cidr, err)
	}
	b, err := json.Marshal(ipnet)
	if err != nil {
		return "", fmt.Errorf("marshal IPNet: %w", err)
	}
	return string(b), nil
}

// generateAccountID returns a netbird-style account ID (xid format
// would be ideal but xid isn't in our deps; uuid suffices).
func generateAccountID(siteName string) string {
	return fmt.Sprintf("acc_%s_%s", siteName, strings.ReplaceAll(uuid.New().String()[:8], "-", ""))
}

// =============================================================================
// create-account
// =============================================================================

func cmdCreateAccount(dsn *string) *cobra.Command {
	var (
		owner          string
		siteName       string
		network        string
		domain         string
		idempotencyKey string
	)
	cmd := &cobra.Command{
		Use:   "create-account",
		Short: "Create a properly-formed netbird account row",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "create-account",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runCreateAccount(ctx, span, *dsn, owner, siteName, network, domain, idempotencyKey)
				})
		},
	}
	cmd.Flags().StringVar(&owner, "owner", "", "Owner email (required)")
	cmd.Flags().StringVar(&siteName, "site-name", "", "Short site name, e.g. alpha (required)")
	cmd.Flags().StringVar(&network, "network", "100.64.0.0/16", "WG network CIDR for this account")
	cmd.Flags().StringVar(&domain, "domain", "", "Domain (default <site-name>.local)")
	cmd.Flags().StringVar(&idempotencyKey, "idempotency-key", "",
		"Explicit idempotency key (default: sha256 of inputs)")
	return cmd
}

func runCreateAccount(ctx context.Context, span trace.Span, dsn, owner, siteName, network, domain, idempotencyKey string) error {
	if owner == "" || siteName == "" {
		return fmt.Errorf("--owner and --site-name required")
	}
	if network == "" {
		network = "100.64.0.0/16"
	}
	if domain == "" {
		domain = fmt.Sprintf("%s.local", siteName)
	}
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	netNetJSON, err := networkNetJSON(network)
	if err != nil {
		return err
	}

	key := resolveIdempotencyKey(idempotencyKey,
		"create-account", owner, siteName, network, domain)

	result, err := runIdempotent(ctx, db, "create-account", key,
		func(tx *sqlx.Tx) (string, error) {
					accountID := generateAccountID(siteName)
					groupID := fmt.Sprintf("grp_%s_all", siteName)

					// Row contract — every NOT-NULL-expected column populated;
					// nullable *netip.Prefix kept NULL. See identity-architecture.md
					// §11.3 for the contract derivation.
					_, err := tx.ExecContext(ctx, `
INSERT INTO accounts (
  id, created_by, created_at,
  domain, domain_category, is_domain_primary_account,
  network_identifier, network_net, network_dns, network_serial,
  dns_settings_disabled_management_groups,
  settings_peer_login_expiration_enabled, settings_peer_login_expiration,
  settings_peer_inactivity_expiration_enabled, settings_peer_inactivity_expiration,
  settings_regular_users_view_blocked,
  settings_groups_propagation_enabled,
  settings_jwt_groups_enabled, settings_jwt_groups_claim_name, settings_jwt_allow_groups,
  settings_routing_peer_dns_resolution_enabled,
  settings_dns_domain, settings_network_range,
  settings_peer_expose_enabled, settings_peer_expose_groups,
  settings_extra_peer_approval_enabled, settings_extra_user_approval_required,
  settings_extra_integrated_validator, settings_extra_integrated_validator_groups,
  settings_lazy_connection_enabled, settings_auto_update_version, settings_auto_update_always,
  reverse_proxy_free_domain_nonce
) VALUES (
  $1, $2, NOW(),
  $3, 'private', true,
  $4, $5, $3, 1,
  '[]',
  false, 86400000000000,
  false, 600000000000,
  false,
  false,
  false, '', '[]',
  false,
  '', NULL,
  false, '[]',
  false, false,
  '', '[]',
  false, 'disabled', false,
  ''
) ON CONFLICT (id) DO NOTHING`, accountID, owner, domain, fmt.Sprintf("%s-net", siteName), netNetJSON)
					if err != nil {
						return "", fmt.Errorf("insert account: %w", err)
					}

					// Default 'All' group — required by mgmt server for peer enrollment.
					_, err = tx.ExecContext(ctx, `
INSERT INTO groups (id, account_id, name, issued)
  VALUES ($1, $2, 'All', 'api')
  ON CONFLICT (id) DO NOTHING`, groupID, accountID)
					if err != nil {
						return "", fmt.Errorf("insert all group: %w", err)
					}

					// No policy (ADR 1185). This CLI used to seed an enabled
					// 'All'->'All' accept policy, pol_<site>_default, mirroring
					// NetBird's own account creation. No goatnet may carry one: peers
					// get reach only from the declared ACL, which goat-standup.sh
					// applies at phase 13 before anything crosses the mesh. Until
					// then enrolled peers correctly reach nothing (status 0/0).

			return fmt.Sprintf("account_id: %s\ngroup_id:   %s\n", accountID, groupID), nil
		})
	if errors.Is(err, errAlreadyApplied) {
		emitAlreadyApplied(key, result)
		span.SetAttributes(attribute.Int64(attrRecordsAffected, 0))
		return err
	}
	if err != nil {
		return err
	}
	fmt.Print(result)
	// Account row + default "All" group row.
	span.SetAttributes(attribute.Int64(attrRecordsAffected, 2))
	return nil
}

// =============================================================================
// create-setup-key
// =============================================================================

func cmdCreateSetupKey(dsn *string) *cobra.Command {
	var (
		accountID      string
		role           string
		ttl            string
		name           string
		usage          int
		idempotencyKey string
		autoGroups     []string
	)
	cmd := &cobra.Command{
		Use:   "create-setup-key",
		Short: "Create a netbird setup key with proper hash format",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "create-setup-key",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runCreateSetupKey(ctx, span, *dsn, accountID, role, ttl, name, usage, idempotencyKey, autoGroups)
				})
		},
	}
	cmd.Flags().StringVar(&accountID, "account", "", "Account ID (required)")
	cmd.Flags().StringVar(&role, "role", "peer", "Role: bootstrap|admin|operator|peer")
	cmd.Flags().StringVar(&ttl, "ttl", "", "Time-to-live (e.g. 24h, 168h, 720h, 2160h). Required.")
	cmd.Flags().StringVar(&name, "name", "", "Display name for this key")
	cmd.Flags().IntVar(&usage, "usage", 200, "Usage limit (0 = unlimited; default 200)")
	cmd.Flags().StringVar(&idempotencyKey, "idempotency-key", "",
		"Explicit idempotency key (default: sha256 of inputs)")
	cmd.Flags().StringSliceVar(&autoGroups, "auto-groups", nil,
		"NetBird group IDs to auto-assign enrolling peers to (written to setup_keys.auto_groups "+
			"as a JSON array). Empty means '[]' — the peer lands in the implicit All group ONLY, "+
			"which grants no peers; a mgmt host must be given its site's INFRA group id "+
			"(read it off a live peer, see node-addition.md step 8).")
	return cmd
}

func runCreateSetupKey(ctx context.Context, span trace.Span, dsn, accountID, role, ttl, name string, usage int, idempotencyKey string, autoGroups []string) error {
	if accountID == "" || role == "" {
		return fmt.Errorf("--account and --role required")
	}
	d, err := time.ParseDuration(ttl)
	if err != nil {
		return fmt.Errorf("parse --ttl %q: %w", ttl, err)
	}
	if name == "" {
		name = fmt.Sprintf("%s-key-%s", role, time.Now().UTC().Format("20060102-150405"))
	}
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	// Auto-derived key uses fields that uniquely identify this key from the
	// operator's intent — account, role, name, ttl, usage. `name` defaults to
	// a timestamp so two no-arg invocations get different keys; if the
	// operator pins --name they get idempotent retries for free.
	key := resolveIdempotencyKey(idempotencyKey,
		"create-setup-key", accountID, role, name, ttl,
		fmt.Sprintf("%d", usage), strings.Join(autoGroups, ","))

	result, err := runIdempotent(ctx, db, "create-setup-key", key,
		func(tx *sqlx.Tx) (string, error) {
			plaintext := strings.ToUpper(uuid.New().String())
			keyHash := hashSetupKey(plaintext)
			keySecret := hiddenKey(plaintext)
			skID := fmt.Sprintf("sk_%s", strings.ReplaceAll(uuid.New().String()[:12], "-", ""))

			// auto_groups is JSON-encoded text and must never be NULL — the
			// mgmt API round-trips the stored value verbatim and a null crashes
			// the dashboard /groups page.
			// It defaults to "[]", which puts the peer in the implicit All group
			// ONLY — and All grants no peers, so a node enrolled with an empty
			// auto_groups sees 0/0 peers. A mgmt host must be
			// minted with --auto-groups <site>-INFRA (node-addition.md step 8).
			autoGroupsJSON := "[]"
			if len(autoGroups) > 0 {
				b, mErr := json.Marshal(autoGroups)
				if mErr != nil {
					return "", fmt.Errorf("marshal --auto-groups %v: %w", autoGroups, mErr)
				}
				autoGroupsJSON = string(b)
			}
			_, err := tx.ExecContext(ctx, `
INSERT INTO setup_keys (
  id, account_id, key, key_secret, name, type,
  created_at, expires_at, updated_at,
  revoked, used_times, usage_limit, ephemeral,
  auto_groups
) VALUES (
  $1, $2, $3, $4, $5, 'reusable',
  NOW(), NOW() + $6::interval, NOW(),
  false, 0, $7, false,
  $8
) ON CONFLICT (id) DO NOTHING`, skID, accountID, keyHash, keySecret, name,
				fmt.Sprintf("%d seconds", int(d.Seconds())), usage, autoGroupsJSON)
			if err != nil {
				return "", fmt.Errorf("insert setup_key: %w", err)
			}

			// First-run output INCLUDES the plaintext (shown once). The
			// recorded `result` does NOT — replay shows only the
			// setup_key_id so plaintext never persists to
			// bootstrap_idempotency.
			fmt.Printf("setup_key_id: %s\n", skID)
			fmt.Printf("plaintext:    %s\n", plaintext)
			fmt.Printf("expires_in:   %s\n", d)
			fmt.Println()
			fmt.Println("IMPORTANT: plaintext is shown ONCE. Hand it to the peer; the DB only")
			fmt.Println("stores the SHA256 hash. Use it with: netbird up --setup-key <plaintext>")
			return fmt.Sprintf("setup_key_id: %s\n", skID), nil
		})
	if errors.Is(err, errAlreadyApplied) {
		emitAlreadyApplied(key, result)
		span.SetAttributes(attribute.Int64(attrRecordsAffected, 0))
		return err
	}
	if err != nil {
		return err
	}
	span.SetAttributes(attribute.Int64(attrRecordsAffected, 1))
	return nil
}

// =============================================================================
// rotate-setup-key
// =============================================================================

func cmdRotateSetupKey(dsn *string) *cobra.Command {
	var (
		skID           string
		idempotencyKey string
	)
	cmd := &cobra.Command{
		Use:   "rotate-setup-key",
		Short: "Generate fresh plaintext for an existing setup-key row",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "rotate-setup-key",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runRotateSetupKey(ctx, span, *dsn, skID, idempotencyKey)
				})
		},
	}
	cmd.Flags().StringVar(&skID, "id", "", "Setup-key id (required)")
	cmd.Flags().StringVar(&idempotencyKey, "idempotency-key", "",
		"Explicit idempotency key (default: sha256 of inputs)")
	return cmd
}

func runRotateSetupKey(ctx context.Context, span trace.Span, dsn, skID, idempotencyKey string) error {
	if skID == "" {
		return fmt.Errorf("--id required")
	}
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	// Auto-derived key is just the sk_id — re-running rotate against the
	// same key is a no-op by default (avoids accidental double-rotation
	// losing the freshly-handed-out plaintext). Operator wanting a
	// deliberate second rotation passes a fresh --idempotency-key.
	key := resolveIdempotencyKey(idempotencyKey, "rotate-setup-key", skID)

	var rowsAffected int64
	result, err := runIdempotent(ctx, db, "rotate-setup-key", key,
		func(tx *sqlx.Tx) (string, error) {
			plaintext := strings.ToUpper(uuid.New().String())
			keyHash := hashSetupKey(plaintext)
			keySecret := hiddenKey(plaintext)

			res, err := tx.ExecContext(ctx, `
UPDATE setup_keys SET key=$1, key_secret=$2, updated_at=NOW(), used_times=0
 WHERE id=$3`, keyHash, keySecret, skID)
			if err != nil {
				return "", fmt.Errorf("update setup_key: %w", err)
			}
			n, err := res.RowsAffected()
			if err != nil {
				return "", fmt.Errorf("count rotated setup keys: %w", err)
			}
			if n == 0 {
				return "", fmt.Errorf("no setup_key with id %s", skID)
			}
			rowsAffected = n
			fmt.Printf("setup_key_id: %s\n", skID)
			fmt.Printf("new plaintext: %s\n", plaintext)
			// Recorded result excludes the plaintext.
			return fmt.Sprintf("setup_key_id: %s\n", skID), nil
		})
	if errors.Is(err, errAlreadyApplied) {
		emitAlreadyApplied(key, result)
		span.SetAttributes(attribute.Int64(attrRecordsAffected, 0))
		return err
	}
	if err != nil {
		return err
	}
	span.SetAttributes(attribute.Int64(attrRecordsAffected, rowsAffected))
	return nil
}

// =============================================================================
// revoke-peer
// =============================================================================

func cmdRevokePeer(dsn *string) *cobra.Command {
	var (
		peerID         string
		idempotencyKey string
	)
	cmd := &cobra.Command{
		Use:   "revoke-peer",
		Short: "Mark a peer revoked (deletes the row)",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "revoke-peer",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runRevokePeer(ctx, span, *dsn, peerID, idempotencyKey)
				})
		},
	}
	cmd.Flags().StringVar(&peerID, "id", "", "Peer id (required)")
	cmd.Flags().StringVar(&idempotencyKey, "idempotency-key", "",
		"Explicit idempotency key (default: sha256 of inputs)")
	return cmd
}

func runRevokePeer(ctx context.Context, span trace.Span, dsn, peerID, idempotencyKey string) error {
	if peerID == "" {
		return fmt.Errorf("--id required")
	}
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	key := resolveIdempotencyKey(idempotencyKey, "revoke-peer", peerID)

	var rowsAffected int64
	result, err := runIdempotent(ctx, db, "revoke-peer", key,
		func(tx *sqlx.Tx) (string, error) {
			gp, err := tx.ExecContext(ctx,
				`DELETE FROM group_peers WHERE peer_id=$1`, peerID)
			if err != nil {
				return "", fmt.Errorf("delete group_peers: %w", err)
			}
			gpN, err := gp.RowsAffected()
			if err != nil {
				return "", fmt.Errorf("count deleted group memberships: %w", err)
			}
			res, err := tx.ExecContext(ctx,
				`DELETE FROM peers WHERE id=$1`, peerID)
			if err != nil {
				return "", fmt.Errorf("delete peer: %w", err)
			}
			n, err := res.RowsAffected()
			if err != nil {
				return "", fmt.Errorf("count deleted peers: %w", err)
			}
			if n == 0 {
				return "", fmt.Errorf("no peer with id %s", peerID)
			}
			rowsAffected = gpN + n
			return fmt.Sprintf("revoked peer %s\n", peerID), nil
		})
	if errors.Is(err, errAlreadyApplied) {
		emitAlreadyApplied(key, result)
		span.SetAttributes(attribute.Int64(attrRecordsAffected, 0))
		return err
	}
	if err != nil {
		return err
	}
	fmt.Print(result)
	span.SetAttributes(attribute.Int64(attrRecordsAffected, rowsAffected))
	return nil
}

// =============================================================================
// list-peers
// =============================================================================

func cmdListPeers(dsn *string) *cobra.Command {
	var accountID string
	cmd := &cobra.Command{
		Use:   "list-peers",
		Short: "List peers in an account",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "list-peers",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runListPeers(ctx, span, *dsn, accountID)
				})
		},
	}
	cmd.Flags().StringVar(&accountID, "account", "", "Account ID (empty = all)")
	return cmd
}

func runListPeers(ctx context.Context, span trace.Span, dsn, accountID string) error {
	_ = ctx
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	// sqlclosecheck false positive (Block 50A.3): database/sql guarantees
	// rows == nil whenever err != nil, so the early return above can't
	// leak; the defer below covers all subsequent paths.
	//nolint:sqlclosecheck // see comment above
	rows, err := db.Queryx(`
SELECT id, name, ip, dns_label, peer_status_connected, peer_status_last_seen
FROM peers WHERE ($1='' OR account_id=$1) ORDER BY name`, accountID)
	if err != nil {
		return err
	}
	defer rows.Close()

	fmt.Printf("%-22s %-24s %-18s %-22s %-10s %s\n",
		"ID", "NAME", "IP", "DNS_LABEL", "CONNECTED", "LAST_SEEN")
	var n int64
	for rows.Next() {
		var id, name, ip, dnsLabel string
		var connected bool
		var lastSeen time.Time
		if err := rows.Scan(&id, &name, &ip, &dnsLabel, &connected, &lastSeen); err != nil {
			return err
		}
		fmt.Printf("%-22s %-24s %-18s %-22s %-10v %s\n",
			id, name, ip, dnsLabel, connected, lastSeen.UTC().Format(time.RFC3339))
		n++
	}
	span.SetAttributes(attribute.Int64(attrRecordsAffected, n))
	return rows.Err()
}

// =============================================================================
// list-setup-keys
// =============================================================================

func cmdListSetupKeys(dsn *string) *cobra.Command {
	var accountID string
	cmd := &cobra.Command{
		Use:   "list-setup-keys",
		Short: "List setup keys in an account",
		RunE: func(cmd *cobra.Command, _ []string) error {
			return withSpan(cmd.Context(), "list-setup-keys",
				[]attribute.KeyValue{attribute.String(attrDSNHost, dsnHost(*dsn))},
				func(ctx context.Context, span trace.Span) error {
					return runListSetupKeys(ctx, span, *dsn, accountID)
				})
		},
	}
	cmd.Flags().StringVar(&accountID, "account", "", "Account ID (empty = all)")
	return cmd
}

func runListSetupKeys(ctx context.Context, span trace.Span, dsn, accountID string) error {
	_ = ctx
	db, err := connect(dsn)
	if err != nil {
		return err
	}
	defer db.Close()

	// sqlclosecheck false positive (Block 50A.3): same as cmdListPeers —
	// database/sql guarantees rows == nil on err, so the defer below
	// covers every read path.
	//nolint:sqlclosecheck // see comment above
	rows, err := db.Queryx(`
SELECT id, name, key_secret, type, expires_at, revoked, used_times, usage_limit
FROM setup_keys WHERE ($1='' OR account_id=$1) ORDER BY created_at DESC`, accountID)
	if err != nil {
		return err
	}
	defer rows.Close()

	fmt.Printf("%-22s %-32s %-12s %-10s %-22s %-7s %-5s %-5s\n",
		"ID", "NAME", "MASK", "TYPE", "EXPIRES", "REVOKED", "USED", "LIMIT")
	var n int64
	for rows.Next() {
		var id, name, mask, kind string
		var revoked bool
		var used, limit int
		var expires time.Time
		if err := rows.Scan(&id, &name, &mask, &kind, &expires, &revoked, &used, &limit); err != nil {
			return err
		}
		fmt.Printf("%-22s %-32s %-12s %-10s %-22s %-7v %-5d %-5d\n",
			id, name, mask, kind, expires.UTC().Format(time.RFC3339), revoked, used, limit)
		n++
	}
	span.SetAttributes(attribute.Int64(attrRecordsAffected, n))
	return rows.Err()
}

// rand reader for token bytes (currently unused but kept for future expansion).
var _ = rand.Reader
