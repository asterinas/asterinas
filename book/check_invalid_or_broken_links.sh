#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

# Temporary files:
# - JSON_FILE stores lychee JSON output.
# - URLS_FILE stores failed http/https links extracted from lychee output.
# - CACHE_UPDATES_FILE stores recovered links to write back into .lycheecache.
# - CACHE_MERGED_FILE is the merged cache output before replace.
JSON_FILE="$(mktemp /tmp/lychee-result-XXXXXX.json)"
URLS_FILE="$(mktemp /tmp/lychee-failed-urls-XXXXXX.txt)"
CACHE_UPDATES_FILE="$(mktemp /tmp/lychee-cache-updates-XXXXXX.txt)"
CACHE_MERGED_FILE="$(mktemp /tmp/lychee-cache-merged-XXXXXX.txt)"

cleanup() {
  rm -f "$JSON_FILE" "$URLS_FILE" "$CACHE_UPDATES_FILE" "$CACHE_MERGED_FILE"
}
trap cleanup EXIT

print_lychee_summary() {
  echo "lychee summary:"
  echo "  total.......$LYCHEE_TOTAL"
  echo "  unique......$LYCHEE_UNIQUE"
  echo "  successful..$LYCHEE_SUCCESSFUL"
  echo "  timeouts....$LYCHEE_TIMEOUTS"
  echo "  redirected..$LYCHEE_REDIRECTS"
  echo "  excluded....$LYCHEE_EXCLUDED"
  echo "  unknown.....$LYCHEE_UNKNOWN"
  echo "  errors......$LYCHEE_ERRORS"
  echo "  unsupported.$LYCHEE_UNSUPPORTED"
}

# Run lychee once and capture JSON output for fallback processing.
echo "[1/3] Running lychee..."
LYCHEE_EXIT=0
if ! lychee --config lychee.toml --format json --no-progress src/ >"$JSON_FILE"; then
  LYCHEE_EXIT=$?
fi

# Collect unique failed http/https links from lychee error/timeout maps.
jq -r '
  [
    (.error_map // {} | to_entries[]? | .value[]? | .url?),
    (.timeout_map // {} | to_entries[]? | .value[]? | .url?)
  ]
  | map(select(type == "string" and test("^https?://")))
  | unique
  | .[]
' "$JSON_FILE" >"$URLS_FILE"

# Print lychee raw stats (same data source as detailed format summary).
LYCHEE_TOTAL="$(jq -r '.total // 0' "$JSON_FILE")"
LYCHEE_UNIQUE="$(jq -r '.unique // 0' "$JSON_FILE")"
LYCHEE_SUCCESSFUL="$(jq -r '.successful // 0' "$JSON_FILE")"
LYCHEE_TIMEOUTS="$(jq -r '.timeouts // 0' "$JSON_FILE")"
LYCHEE_REDIRECTS="$(jq -r '.redirects // 0' "$JSON_FILE")"
LYCHEE_EXCLUDED="$(jq -r '.excludes // 0' "$JSON_FILE")"
LYCHEE_UNKNOWN="$(jq -r '.unknown // 0' "$JSON_FILE")"
LYCHEE_ERRORS="$(jq -r '.errors // 0' "$JSON_FILE")"
LYCHEE_UNSUPPORTED="$(jq -r '.unsupported // 0' "$JSON_FILE")"

TOTAL_FAILED_BY_LYCHEE="$(wc -l <"$URLS_FILE" | tr -d ' ')"
echo "lychee failed links: $TOTAL_FAILED_BY_LYCHEE"
if [ "$TOTAL_FAILED_BY_LYCHEE" -eq 0 ]; then
  print_lychee_summary
  exit "$LYCHEE_EXIT"
fi

# Retry failed links using plain curl; report recovered/failed links.
echo "[2/3] Curl fallback on lychee-failed links..."
RECOVERED=0
FAILED=0
while IFS= read -r url; do
  [ -z "$url" ] && continue
  STATUS_CODE="$(curl -o /dev/null -w '%{http_code}' "$url" 2>/dev/null)"
  CURL_EXIT=$?
  if [ "$CURL_EXIT" -eq 0 ]; then
    RECOVERED=$((RECOVERED + 1))
    echo "RECOVERED: $url (status=$STATUS_CODE)"
    # Status code is recorded as 200 in .lycheecache to let lychee skip the link next time.
    printf '%s,%s,%s\n' "$url" "200" "$(date +%s)" >>"$CACHE_UPDATES_FILE"
  else
    FAILED=$((FAILED + 1))
    echo "FAILED:    $url"
  fi
done <"$URLS_FILE"

# Write recovered links into .lycheecache as url,status,timestamp.
UPDATED_CACHE_LINES=0
if [ -s "$CACHE_UPDATES_FILE" ]; then
  if [ -f .lycheecache ]; then
    # Merge cache with URL-level deduplication:
    # - First pass (updates): keep the latest recovered record by URL.
    # - Second pass (existing cache): keep only URLs not present in updates.
    # - End: append updated records so recovered entries replace old ones.
    awk -F, '
      NR==FNR { new[$1]=$0; order[++n]=$1; next }
      !($1 in new) { print }
      END { for (i=1; i<=n; i++) print new[order[i]] }
    ' "$CACHE_UPDATES_FILE" .lycheecache >"$CACHE_MERGED_FILE"
  else
    cat "$CACHE_UPDATES_FILE" >"$CACHE_MERGED_FILE"
  fi
  mv "$CACHE_MERGED_FILE" .lycheecache
  UPDATED_CACHE_LINES="$(wc -l <"$CACHE_UPDATES_FILE" | tr -d ' ')"
fi

echo "[3/3] Summary: lychee_failed=$TOTAL_FAILED_BY_LYCHEE recovered=$RECOVERED still_failed=$FAILED"
echo "[3/3] Cache updated lines: $UPDATED_CACHE_LINES"
print_lychee_summary
if [ "$FAILED" -gt 0 ]; then
  exit 1
fi
exit 0
