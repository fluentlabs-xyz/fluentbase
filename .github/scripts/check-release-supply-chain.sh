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

# --- Canonical release-tag guards (independent of the workflow argument) ---

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The tag grammar must keep rejecting invalid suffixes (v1.4.0oops, v1.4.00, ...).
if ! "${script_dir}/check-release-tag.sh" --self-test >&2; then
  echo "check-release-tag.sh self-test failed" >&2
  fail=1
fi

# Every pipeline that signs artifacts, drafts releases, publishes crates, or
# moves Docker channels must call the shared canonical tag validation.
for guarded in .github/workflows/release.yml .github/workflows/publish.yml \
  .github/workflows/build-docker.yml .github/workflows/docker.yml Makefile; do
  if ! grep -q 'check-release-tag\.sh' "$guarded"; then
    echo "${guarded}: missing canonical release-tag validation (check-release-tag.sh)" >&2
    fail=1
  fi
done

# In release.yml, every job that builds, signs, uploads, or drafts a release
# must sit behind check-version in the job DAG, so a noncanonical tag cannot
# produce externally visible artifacts even when the workflow fails overall.
release_workflow=.github/workflows/release.yml
for job in build-genesis build draft-release; do
  needs="$(awk -v job="$job" '
    $0 == "  " job ":" { injob = 1; next }
    injob && /^  [A-Za-z0-9_-]+:/ { injob = 0 }
    injob && /^    needs:/ { print; exit }
  ' "$release_workflow")"
  if [[ "$needs" != *check-version* ]]; then
    echo "${release_workflow}: job '${job}' must depend on check-version (found: ${needs:-no needs line})" >&2
    fail=1
  fi
done

exit "$fail"
