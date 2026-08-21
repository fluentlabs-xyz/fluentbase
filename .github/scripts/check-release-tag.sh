#!/usr/bin/env bash
# Canonical release-tag validation, shared by every pipeline that signs
# artifacts, drafts releases, publishes crates, or moves Docker channels
# (release.yml, publish.yml, docker.yml, build-docker.yml, Makefile).
#
# Usage:
#   check-release-tag.sh <tag> [cargo-toml]   validate a tag against the
#                                             workspace version (default ./Cargo.toml)
#   check-release-tag.sh --self-test          run the built-in guard tests
#
# The only accepted forms are:
#   v<workspace-version>          -> channel=stable
#   v<workspace-version>-rc.N     -> channel=prerelease (N >= 1, no leading zeros)
# and when the workspace version is itself an rc (X.Y.Z-rc.N), only the exact
# tag v<workspace-version> is accepted (channel=prerelease).
#
# On success the resolved channel is printed to stdout as `channel=<...>` and
# appended to $GITHUB_OUTPUT when set. Anything else fails with exit 1: a typo
# such as v1.4.0oops or v1.4.00 must never reach signing, release drafts,
# crates.io publishing, or the Docker `latest` channel.
set -euo pipefail

readonly NUM='(0|[1-9][0-9]*)'
readonly BASE_RE="^${NUM}\.${NUM}\.${NUM}$"
readonly RC_RE='^rc\.([1-9][0-9]*)$'

validate() {
  local tag="$1" cargo_ver="$2" base rc channel

  base="$cargo_ver"
  rc=""
  if [[ "$cargo_ver" == *-* ]]; then
    base="${cargo_ver%%-*}"
    rc="${cargo_ver#*-}"
  fi
  if ! [[ "$base" =~ $BASE_RE ]] || { [[ -n "$rc" ]] && ! [[ "$rc" =~ $RC_RE ]]; }; then
    echo "workspace version '$cargo_ver' is not canonical (X.Y.Z or X.Y.Z-rc.N)" >&2
    return 1
  fi

  if [[ "$tag" == "v$cargo_ver" ]]; then
    channel="stable"
    if [[ -n "$rc" ]]; then
      channel="prerelease"
    fi
  elif [[ -z "$rc" && "$tag" == "v$cargo_ver-rc."* ]] \
    && [[ "${tag#"v$cargo_ver-"}" =~ $RC_RE ]]; then
    channel="prerelease"
  else
    echo "tag '$tag' is not canonical for workspace version '$cargo_ver'" >&2
    if [[ -z "$rc" ]]; then
      echo "accepted forms: v${cargo_ver} or v${cargo_ver}-rc.N (N >= 1)" >&2
    else
      echo "accepted form: v${cargo_ver}" >&2
    fi
    return 1
  fi

  echo "channel=$channel"
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "channel=$channel" >> "$GITHUB_OUTPUT"
  fi
}

self_test() {
  local failures=0

  expect() {
    local want="$1" tag="$2" cargo_ver="$3" out status
    out="$(GITHUB_OUTPUT= validate "$tag" "$cargo_ver" 2>/dev/null)" && status=0 || status=$?
    if [[ "$want" == "reject" ]]; then
      if [[ "$status" -eq 0 ]]; then
        echo "self-test: expected rejection of tag '$tag' (workspace $cargo_ver), got '$out'" >&2
        failures=$((failures + 1))
      fi
    elif [[ "$status" -ne 0 || "$out" != "channel=$want" ]]; then
      echo "self-test: expected tag '$tag' (workspace $cargo_ver) -> $want, got status=$status output='$out'" >&2
      failures=$((failures + 1))
    fi
  }

  expect stable     v1.4.0        1.4.0
  expect stable     v0.3.7        0.3.7
  expect prerelease v1.4.0-rc.1   1.4.0
  expect prerelease v1.4.0-rc.10  1.4.0
  expect prerelease v1.4.0-rc.2   1.4.0-rc.2

  # Invalid suffixes and typos must never pass.
  expect reject v1.4.0oops     1.4.0
  expect reject v1.4.00        1.4.0
  expect reject v1.4.0.1       1.4.0
  expect reject v1.4.1         1.4.0
  expect reject 1.4.0          1.4.0
  expect reject vv1.4.0        1.4.0
  expect reject v1.4.0-dev     1.4.0
  expect reject v1.4.0-rc      1.4.0
  expect reject v1.4.0-rc1     1.4.0
  expect reject v1.4.0-rc.0    1.4.0
  expect reject v1.4.0-rc.01   1.4.0
  expect reject v1.4.0-rc.1.1  1.4.0
  # A workspace still on an rc accepts only its exact tag.
  expect reject v1.4.0         1.4.0-rc.2
  expect reject v1.4.0-rc.3    1.4.0-rc.2
  expect reject v1.4.0-rc.2-rc.1 1.4.0-rc.2
  # Non-canonical workspace versions are refused outright.
  expect reject v01.4.0        01.4.0
  expect reject v1.4.0-nightly 1.4.0-nightly

  if [[ "$failures" -ne 0 ]]; then
    echo "check-release-tag.sh self-test: $failures case(s) failed" >&2
    return 1
  fi
  echo "check-release-tag.sh self-test: all cases passed"
}

main() {
  if [[ "${1:-}" == "--self-test" ]]; then
    self_test
    return
  fi
  if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 <tag> [cargo-toml] | $0 --self-test" >&2
    return 2
  fi
  local tag="$1" cargo_toml="${2:-Cargo.toml}" cargo_ver
  cargo_ver="$(grep -m1 '^version' "$cargo_toml" | awk -F'=' '{print $2}' | tr -d ' "')"
  if [[ -z "$cargo_ver" ]]; then
    echo "unable to read workspace version from $cargo_toml" >&2
    return 1
  fi
  validate "$tag" "$cargo_ver"
}

main "$@"
