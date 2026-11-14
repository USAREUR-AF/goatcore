# shellcheck shell=bash disable=SC2034  # sourced by ../run.sh, which reads EXPECT
# ADR 1205 D10: only the hub makes a platform admin. Site A is the hub.
# A platform admin is an identity whose metadata_public.groups holds
# PLATFORM-ADMIN, the admin-managed groups (ADR 1182).
#
# Cut off: B's own admin API refuses to grant PLATFORM-ADMIN to X, because
# B is not the hub. B then writes grants with its guard disabled, as a site
# broken into could: an UPDATE giving X PLATFORM-ADMIN, and a new identity W
# created with it. The hub grants Y. After heal, A has applied both of B's
# rows without PLATFORM-ADMIN, the other groups kept, and logged each in
# goat_ceiling_refused with the groups as received and B's commit timestamp;
# B has the hub's grant to Y.
# Then B's subscription from A is set to forward origins, and the hub grants
# F: B cannot tell the node that made a forwarded commit, so it applies the
# hub's grant without PLATFORM-ADMIN and logs it. Forwarding is put back.
# Last, the hub removes the group from X, W and F, a removal any site may
# make, and the sites agree.

EX="pa-x-$RUN@harness.test"
EY="pa-y-$RUN@harness.test"
EW="pa-w-$RUN@harness.test"
EF="pa-f-$RUN@harness.test"
EXPECT=""
PA_GROUPS='{"groups":["harness-staff","PLATFORM-ADMIN"]}'

seed() {
  kratos_create a "$EX" "PA X" > "$SCRATCH/x"
  kratos_create a "$EY" "PA Y" > "$SCRATCH/y"
  kratos_create a "$EF" "PA F" > "$SCRATCH/f"
}

guard_off() { # guard_off <site>
  kr "$1" "DO \$\$ BEGIN
             IF EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'goat_platform_admin_ceiling') THEN
               ALTER TABLE identities DISABLE TRIGGER goat_platform_admin_ceiling;
             END IF;
           END \$\$;" >/dev/null
}
guard_on() { # guard_on <site>
  kr "$1" "DO \$\$ BEGIN
             IF EXISTS (SELECT 1 FROM pg_trigger WHERE tgname = 'goat_platform_admin_ceiling') THEN
               ALTER TABLE identities ENABLE ALWAYS TRIGGER goat_platform_admin_ceiling;
             END IF;
           END \$\$;" >/dev/null
}
commit_ts() { # commit_ts <site> <identity id>  -> the commit timestamp of the row's last write, epoch seconds
  kr "$1" "SELECT extract(epoch FROM pg_xact_commit_timestamp(xmin)) FROM identities WHERE id = '$2';"
}
has_log() { # has_log <site>  -> t when the site has goat_ceiling_refused
  kr "$1" "SELECT to_regclass('goat_ceiling_refused') IS NOT NULL;"
}
refused() { # refused <site> <identity id> <op> <origin> <reason>  -> how many goat_ceiling_refused rows match
  [[ "$(has_log "$1")" == t ]] || { echo "no goat_ceiling_refused table"; return; }
  kr "$1" "SELECT count(*) FROM goat_ceiling_refused
            WHERE identity_id = '$2' AND op = '$3' AND origin = '$4' AND reason = '$5';"
}
received() { # received <site> <identity id>  -> "<groups as received> <commit timestamp, epoch seconds>"
  [[ "$(has_log "$1")" == t ]] || { echo "no goat_ceiling_refused table"; return; }
  kr "$1" "SELECT goat_ceiling_refused_stamp();" >/dev/null
  kr "$1" "SELECT string_agg(groups_received::text || ' ' || coalesce(extract(epoch FROM committed_at)::text, 'unstamped'), ';')
             FROM goat_ceiling_refused WHERE identity_id = '$2';"
}
groups_of() { # groups_of <site> <identity id>
  kr "$1" "SELECT coalesce(metadata_public->'groups', 'null'::jsonb)::text FROM identities WHERE id = '$2';"
}

write_b() {
  local x; x=$(cat "$SCRATCH/x")
  if kapi b PATCH "/admin/identities/$x" \
       "[{\"op\":\"add\",\"path\":\"/metadata_public\",\"value\":$PA_GROUPS}]" >/dev/null 2>&1; then
    echo accepted > "$SCRATCH/b-api"
  else
    echo refused > "$SCRATCH/b-api"
  fi
  # A site broken into bypasses its own guard: an UPDATE and an INSERT.
  guard_off b
  kr b "UPDATE identities SET metadata_public = '$PA_GROUPS'::jsonb, updated_at = now() WHERE id = '$x';" >/dev/null
  kratos_create b "$EW" "PA W" "$PA_GROUPS" > "$SCRATCH/w"
  guard_on b
  commit_ts b "$x" > "$SCRATCH/x-ts"
  commit_ts b "$(cat "$SCRATCH/w")" > "$SCRATCH/w-ts"
}

write_a() {
  kapi a PATCH "/admin/identities/$(cat "$SCRATCH/y")" \
    "[{\"op\":\"add\",\"path\":\"/metadata_public\",\"value\":$PA_GROUPS}]" >/dev/null
}

after_heal() {
  local x w f
  x=$(cat "$SCRATCH/x"); w=$(cat "$SCRATCH/w"); f=$(cat "$SCRATCH/f")
  groups_of a "$x" > "$SCRATCH/a-x-groups"
  groups_of a "$w" > "$SCRATCH/a-w-groups"
  refused a "$x" UPDATE site_b not_hub > "$SCRATCH/a-x-refused"
  refused a "$w" INSERT site_b not_hub > "$SCRATCH/a-w-refused"
  received a "$x" > "$SCRATCH/a-x-received"
  received a "$w" > "$SCRATCH/a-w-received"

  # B forwards origins from A; the hub's grant to F arrives at B stripped.
  kr b "UPDATE spock.subscription SET sub_forward_origins = '{all}' WHERE sub_name = 'sub_a_to_b';" >/dev/null
  kapi a PATCH "/admin/identities/$f" \
    "[{\"op\":\"add\",\"path\":\"/metadata_public\",\"value\":$PA_GROUPS}]" >/dev/null
  sync_all
  groups_of b "$f" > "$SCRATCH/b-f-groups"
  refused b "$f" UPDATE site_a forwarding > "$SCRATCH/b-f-refused"
  kr b "UPDATE spock.subscription SET sub_forward_origins = '{}' WHERE sub_name = 'sub_a_to_b';" >/dev/null
  sync_all

  # The hub's remedy: X, W and F written again with no PLATFORM-ADMIN.
  for id in "$x" "$w" "$f"; do
    kapi a PATCH "/admin/identities/$id" \
      '[{"op":"add","path":"/metadata_public","value":{"groups":["harness-staff"]}}]' >/dev/null
  done
}

check() {
  local x y w f
  x=$(cat "$SCRATCH/x"); y=$(cat "$SCRATCH/y"); w=$(cat "$SCRATCH/w"); f=$(cat "$SCRATCH/f")
  expect_eq "b: its own admin API granting PLATFORM-ADMIN" "$(cat "$SCRATCH/b-api")" refused
  expect_eq "a: X's groups when B's UPDATE arrives" "$(cat "$SCRATCH/a-x-groups")" '["harness-staff"]'
  expect_eq "a: W's groups when B's INSERT arrives" "$(cat "$SCRATCH/a-w-groups")" '["harness-staff"]'
  expect_eq "a: refused UPDATE grants logged for X from site_b" "$(cat "$SCRATCH/a-x-refused")" 1
  expect_eq "a: refused INSERT grants logged for W from site_b" "$(cat "$SCRATCH/a-w-refused")" 1
  expect_eq "a: X's refusal holds the groups as received and B's commit timestamp" \
    "$(cat "$SCRATCH/a-x-received")" "[\"harness-staff\", \"PLATFORM-ADMIN\"] $(cat "$SCRATCH/x-ts")"
  expect_eq "a: W's refusal holds the groups as received and B's commit timestamp" \
    "$(cat "$SCRATCH/a-w-received")" "[\"harness-staff\", \"PLATFORM-ADMIN\"] $(cat "$SCRATCH/w-ts")"
  expect_eq "b: Y holds the hub's grant" "$(pa_admin b "$y")" yes
  expect_eq "b: F's groups when the hub's grant arrives while B forwards origins" "$(cat "$SCRATCH/b-f-groups")" '["harness-staff"]'
  expect_eq "b: refused grants logged for F from site_a while B forwards origins" "$(cat "$SCRATCH/b-f-refused")" 1
  for s in a b; do
    for id in "$x" "$w" "$f"; do
      expect_eq "$s: $id holds PLATFORM-ADMIN after the hub's removal" "$(pa_admin "$s" "$id")" no
    done
    expect_eq "$s: forward_origins of $s's subscription" \
      "$(kr "$s" "SELECT coalesce(cardinality(sub_forward_origins), 0) FROM spock.subscription;")" 0
  done
}
