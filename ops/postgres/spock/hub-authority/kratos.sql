-- ADR 1205 D10: only the goatnet's hub makes a platform admin.
--
-- A platform admin is an identity whose metadata_public.groups holds
-- "PLATFORM-ADMIN": the admin-managed groups, which are the authority
-- (ADR 1182 item 3). This guard reads only metadata_public->'groups'.
--
-- A grant is a row whose groups hold PLATFORM-ADMIN where the row this site
-- had did not, or a new row that holds it. For a grant:
--   - a local write at the hub: allowed;
--   - a local write at any other Spock site: refused with an error, so the
--     admin API or console that tried it reports it;
--   - a replicated row whose sending node is the hub: applied as is;
--   - a replicated row from any other node: applied without PLATFORM-ADMIN,
--     the rest of the row unchanged, and logged in goat_ceiling_refused.
-- A replicated grant is also treated as not from the hub, and stripped and
-- logged, when any subscription at this site forwards origins (non-empty
-- forward_origins): the sending node is then not always the node that made
-- the commit (ADR 1205 D9 requires forward_origins empty).
-- A site with no Spock node (a goatnet of one site) has no other site, so a
-- local grant there is allowed. A Spock site that names no hub refuses
-- every grant, local or replicated. Removing PLATFORM-ADMIN is never
-- touched: it is a revocation, which any site may make (ADR 1165 class 3).
--
-- The hub's name is read from this site's own configuration, the server
-- setting goat.hub_site, which the ory-narrow-repset role writes from
-- goatnet_hub_site with ALTER SYSTEM (ADR 1205 D8), never from a replicated
-- row. goatnet_hub_site is the hub's Spock node name as spock-mesh-peering
-- names it; ory-narrow-repset names the same site's node in this database
-- <name>_<database>, so either name is the hub here.
--
-- The sending node of a replicated row: inside Spock's apply, the backend's
-- pg_stat_activity.backend_type is "spock apply <database oid>:<subscription
-- id>". The server sets it; a client session reads "client backend", and
-- the client-settable application_name is not used. The subscription's
-- sub_origin is the node it receives from.
--
-- goat_ceiling_refused is in no replication set: each site keeps its own.
-- Each row holds the groups exactly as received and the id of the local
-- transaction that applied the grant. Spock's apply commits that
-- transaction with the sending node's commit timestamp, so once it has
-- committed, goat_ceiling_refused_stamp() copies that timestamp into
-- committed_at. The guard runs it at every refusal, and a reader runs it
-- before reading.
--
-- Installed into the kratos database at every Spock site, after the ADR 1193
-- guard (revocation-guard/kratos.sql). Idempotent.

CREATE TABLE IF NOT EXISTS public.goat_ceiling_refused (
  id              bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  at              timestamptz NOT NULL DEFAULT clock_timestamp(),
  identity_id     uuid        NOT NULL,
  op              text        NOT NULL,
  origin          text        NOT NULL,
  reason          text        NOT NULL CHECK (reason IN ('not_hub', 'forwarding', 'no_hub')),
  groups_received jsonb,
  grant_xid       xid8        NOT NULL DEFAULT pg_current_xact_id(),
  committed_at    timestamptz
);

-- Copies each refused grant's commit timestamp into committed_at once its
-- transaction has committed. Returns the number of rows stamped.
CREATE OR REPLACE FUNCTION goat_ceiling_refused_stamp() RETURNS integer
LANGUAGE plpgsql AS $$
DECLARE
  n integer;
BEGIN
  UPDATE public.goat_ceiling_refused r
     SET committed_at = t.ts
    FROM (SELECT id, pg_xact_commit_timestamp(grant_xid::xid) AS ts
            FROM public.goat_ceiling_refused
           WHERE committed_at IS NULL) t
   WHERE r.id = t.id AND t.ts IS NOT NULL;
  GET DIAGNOSTICS n = ROW_COUNT;
  RETURN n;
END $$;

-- The Spock node a replicated change being applied in this session came
-- from, or NULL for a local write, or 'unknown' when the apply backend's
-- subscription cannot be read.
CREATE OR REPLACE FUNCTION goat_sending_node() RETURNS text
LANGUAGE plpgsql STABLE AS $$
DECLARE
  btype text;
  sub   text;
  node  text;
BEGIN
  IF NOT pg_replication_origin_session_is_setup() THEN
    RETURN NULL;
  END IF;
  SELECT backend_type INTO btype FROM pg_stat_activity WHERE pid = pg_backend_pid();
  sub := substring(btype FROM '^spock apply [0-9]+:([0-9]+)$');
  IF sub IS NULL OR to_regclass('spock.subscription') IS NULL THEN
    RETURN 'unknown';
  END IF;
  EXECUTE 'SELECT n.node_name::text FROM spock.subscription s JOIN spock.node n ON n.node_id = s.sub_origin WHERE s.sub_id = $1::oid'
    INTO node USING sub;
  RETURN coalesce(node, 'unknown');
END $$;

-- This site's own Spock node name, or NULL where Spock has no node here.
CREATE OR REPLACE FUNCTION goat_local_node() RETURNS text
LANGUAGE plpgsql STABLE AS $$
DECLARE
  node text;
BEGIN
  IF to_regclass('spock.local_node') IS NULL THEN
    RETURN NULL;
  END IF;
  EXECUTE 'SELECT n.node_name::text FROM spock.local_node l JOIN spock.node n ON n.node_id = l.node_id' INTO node;
  RETURN node;
END $$;

-- True when any subscription at this site forwards origins.
CREATE OR REPLACE FUNCTION goat_forwards_origins() RETURNS boolean
LANGUAGE plpgsql STABLE AS $$
DECLARE
  f boolean;
BEGIN
  IF to_regclass('spock.subscription') IS NULL THEN
    RETURN false;
  END IF;
  EXECUTE 'SELECT EXISTS (SELECT 1 FROM spock.subscription WHERE coalesce(cardinality(sub_forward_origins), 0) > 0)'
    INTO f;
  RETURN f;
END $$;

CREATE OR REPLACE FUNCTION goat_platform_admin_ceiling() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  hub    text    := nullif(current_setting('goat.hub_site', true), '');
  had    boolean := TG_OP = 'UPDATE'
                    AND coalesce(OLD.metadata_public -> 'groups', '[]'::jsonb) ? 'PLATFORM-ADMIN';
  has    boolean := coalesce(NEW.metadata_public -> 'groups', '[]'::jsonb) ? 'PLATFORM-ADMIN';
  me     text;
  sender text;
  why    text;
BEGIN
  IF NOT has OR had THEN
    RETURN NEW;  -- not a grant
  END IF;
  me := goat_local_node();
  sender := goat_sending_node();
  IF sender IS NULL THEN
    -- A local write.
    IF me IS NULL THEN
      RETURN NEW;  -- no Spock node here: a goatnet of one site
    END IF;
    IF hub IS NULL THEN
      RAISE EXCEPTION 'platform admin refused at %: goat.hub_site is not set at this site; set goatnet_hub_site and run the ory-narrow-repset role, which writes goat.hub_site', me
        USING ERRCODE = 'insufficient_privilege';
    END IF;
    IF me IN (hub, hub || '_' || current_database()) THEN
      RETURN NEW;
    END IF;
    RAISE EXCEPTION 'platform admin refused at %: only the hub (%) makes a platform admin', me, hub
      USING ERRCODE = 'insufficient_privilege';
  END IF;
  -- A replicated row.
  IF hub IS NULL THEN
    why := 'no_hub';
  ELSIF goat_forwards_origins() THEN
    why := 'forwarding';
  ELSIF sender IN (hub, hub || '_' || current_database()) THEN
    RETURN NEW;
  ELSE
    why := 'not_hub';
  END IF;
  INSERT INTO public.goat_ceiling_refused (identity_id, op, origin, reason, groups_received)
    VALUES (NEW.id, TG_OP, sender, why, NEW.metadata_public -> 'groups');
  IF jsonb_typeof(NEW.metadata_public -> 'groups') = 'array' THEN
    NEW.metadata_public := jsonb_set(
      NEW.metadata_public, '{groups}',
      (SELECT coalesce(jsonb_agg(g ORDER BY o), '[]'::jsonb)
         FROM jsonb_array_elements(NEW.metadata_public -> 'groups') WITH ORDINALITY AS e(g, o)
        WHERE g <> to_jsonb('PLATFORM-ADMIN'::text)));
  ELSE
    NEW.metadata_public := jsonb_set(NEW.metadata_public, '{groups}', '[]'::jsonb);
  END IF;
  PERFORM goat_ceiling_refused_stamp();
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS goat_platform_admin_ceiling ON public.identities;
CREATE TRIGGER goat_platform_admin_ceiling BEFORE INSERT OR UPDATE ON public.identities
  FOR EACH ROW EXECUTE FUNCTION goat_platform_admin_ceiling();
ALTER TABLE public.identities ENABLE ALWAYS TRIGGER goat_platform_admin_ceiling;
