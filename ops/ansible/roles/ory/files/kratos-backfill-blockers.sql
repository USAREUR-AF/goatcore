-- Rows that would stop Kratos 26.2.0's identity_id migrations, counted before
-- they run. Read by ops/ansible/ory-kratos-upgrade-window.yml (phase 0) and
-- ops/postgres/spock/tests/kratos-26-guard/run.sh. Read-only.
--
-- Kratos 26.2.0 backfills identity_credential_identifiers.identity_id and
-- session_devices.identity_id (persistence/sql/migrations/go/
-- 20251105000000000000_identity_id_not_null_fks.go), then sets both NOT NULL
-- with a foreign key to identities. A row the backfill cannot fill, or fills
-- with an identity that does not exist, fails that migration halfway through
-- the window. Each class below is such a row:
--
--   identifier_without_credential  no identity_credentials row with the same
--                                  id and nid: the backfill join skips it.
--   identifier_owner_missing       its credential names an identity that is
--                                  gone (a delete applied by Spock runs with
--                                  foreign-key triggers off, so no cascade).
--   identifier_owner_deleted       the owner carries an uncancelled
--                                  kratos.identity.deleted revocation: the
--                                  revocation guard (ADR 1193) returns NULL for
--                                  the backfill's UPDATE, so the row stays NULL.
--   session_device_orphaned        no session with the same id and nid, or the
--                                  session's identity is gone.
--
-- Output: one `class|count` line per class. Every count must be 0. Remedy: run
-- `SELECT goat_revocation_sweep();` (deletes a revoked identity's children),
-- then delete the remaining orphans, then re-run this.
CREATE OR REPLACE FUNCTION pg_temp.kratos_backfill_blockers()
RETURNS TABLE (class text, n bigint) LANGUAGE plpgsql AS $$
DECLARE
  has_col boolean := EXISTS (SELECT 1 FROM information_schema.columns
                              WHERE table_schema = 'public' AND table_name = 'identity_credential_identifiers'
                                AND column_name = 'identity_id');
  has_sd_col boolean := EXISTS (SELECT 1 FROM information_schema.columns
                                 WHERE table_schema = 'public' AND table_name = 'session_devices'
                                   AND column_name = 'identity_id');
  unfilled text := CASE WHEN has_col THEN ' AND ici.identity_id IS NULL' ELSE '' END;
  sd_unfilled text := CASE WHEN has_sd_col THEN ' AND sd.identity_id IS NULL' ELSE '' END;
BEGIN
  class := 'identifier_without_credential';
  EXECUTE 'SELECT count(*) FROM public.identity_credential_identifiers ici
            LEFT JOIN public.identity_credentials ic ON ic.id = ici.identity_credential_id AND ic.nid = ici.nid
           WHERE ic.id IS NULL' || unfilled INTO n;
  RETURN NEXT;

  class := 'identifier_owner_missing';
  EXECUTE 'SELECT count(*) FROM public.identity_credential_identifiers ici
             JOIN public.identity_credentials ic ON ic.id = ici.identity_credential_id AND ic.nid = ici.nid
             LEFT JOIN public.identities i ON i.id = ic.identity_id
           WHERE i.id IS NULL' || unfilled INTO n;
  RETURN NEXT;

  class := 'identifier_owner_deleted';
  n := 0;
  IF to_regprocedure('public.goat_revoked(text,text)') IS NOT NULL THEN
    EXECUTE 'SELECT count(*) FROM public.identity_credential_identifiers ici
               JOIN public.identity_credentials ic ON ic.id = ici.identity_credential_id AND ic.nid = ici.nid
             WHERE public.goat_revoked(''kratos.identity.deleted'', ic.identity_id::text)' || unfilled INTO n;
  END IF;
  RETURN NEXT;

  class := 'session_device_orphaned';
  n := 0;
  IF to_regclass('public.session_devices') IS NOT NULL THEN
    EXECUTE 'SELECT count(*) FROM public.session_devices sd
               LEFT JOIN public.sessions s ON s.id = sd.session_id AND s.nid = sd.nid
               LEFT JOIN public.identities i ON i.id = s.identity_id
             WHERE (s.id IS NULL OR i.id IS NULL)' || sd_unfilled INTO n;
  END IF;
  RETURN NEXT;
END $$;
SELECT class || '|' || n FROM pg_temp.kratos_backfill_blockers();
