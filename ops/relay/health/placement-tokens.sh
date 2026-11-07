#!/usr/bin/env bash
# placement-tokens.sh — emit the ADR 1080 placement token block for a
# relays.conf line, so a new relay's tokens can be pasted rather than typed.
#
#   ./placement-tokens.sh <cloud-region> [az]
#   ./placement-tokens.sh eu-central-1 eu-central-1a
#   -> provider=aws az=eu-central-1a geo=50.11,8.68 place=Frankfurt  # oss-allow: AWS's published city for eu-central-1
#
# The region→approx-coords table below is SEED DATA FOR WRITING CONF LINES,
# never a runtime fallback: the prober/dashboard render only what a conf line
# DECLARES (no geo= -> the relay sits in the "unplaced" tray, ADR 1080
# honesty rule). Coordinates are the provider region's commonly documented
# metro, good enough for a fleet map, not a survey.
set -euo pipefail
REGION="${1:-}"
AZ="${2:-}"
if [ -z "$REGION" ]; then
  echo "usage: $0 <cloud-region> [az]" >&2
  exit 2
fi

# Public AWS region locations (the providers' published cities), used only
# to place a relay dot on the dashboard map.
# region  lat,lon  display (underscores render as spaces in the dashboard)
# Only the first three fields of a row are read.
SEED="
us-east-1      38.95,-77.45   N._Virginia
us-west-2      45.85,-119.70  Oregon
eu-west-1      53.35,-6.27    Dublin
eu-central-1   50.11,8.68     Frankfurt      # oss-allow: AWS's published city for this region
eu-north-1     59.33,18.06    Stockholm
il-central-1   32.08,34.78    Tel_Aviv
ap-south-1     19.08,72.88    Mumbai         # oss-allow: AWS's published city for this region
me-central-1   25.20,55.27    Dubai
"

line=$(printf '%s\n' "$SEED" | awk -v r="$REGION" '$1==r{print $2, $3}')
out="provider=aws"
[ -n "$AZ" ] && out="$out az=$AZ"
if [ -n "$line" ]; then
  geo="${line%% *}"; place="${line##* }"
  out="$out geo=$geo place=$place"
else
  echo "note: region '$REGION' not in the seed table — declare geo=<lat>,<lon> place=<name> by hand (or leave unplaced)" >&2
fi
echo "$out"
