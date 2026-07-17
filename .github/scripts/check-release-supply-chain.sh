#!/usr/bin/env bash
set -euo pipefail

workflow=${1:-.github/workflows/release.yml}

fail=0

while IFS=: read -r line_no ref; do
  if [[ ! "$ref" =~ @[0-9a-f]{40}([[:space:]]|$) ]]; then
    echo "${workflow}:${line_no}: action is not pinned to a full commit SHA: ${ref}" >&2
    fail=1
  fi
done < <(grep -nE '^[[:space:]]*uses:[[:space:]]*[^[:space:]]+' "$workflow" | sed -E 's/^([0-9]+):[[:space:]]*uses:[[:space:]]*/\1:/')

if grep -nE 'cargo[[:space:]]+(b|build|publish|nextest|test|clippy)([[:space:]]|$)' "$workflow" | grep -v -- '--locked' >&2; then
  echo "${workflow}: cargo release invocations must use --locked or call a Make target that does" >&2
  fail=1
fi

for required in 'release-manifest.sh create' 'release-manifest.sh verify' 'sbom' 'provenance'; do
  if ! grep -q "$required" "$workflow"; then
    echo "${workflow}: missing required release supply-chain marker: ${required}" >&2
    fail=1
  fi
done

exit "$fail"
