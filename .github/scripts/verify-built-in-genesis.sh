#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
chainspec="$repo_root/crates/node/src/chainspec.rs"
key_source="$repo_root/crates/release-verify/src/key.rs"
expected_fingerprint=0A6D05E5DD98069BA184ED8304A68D620D5208FD

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

awk '
  /-----BEGIN PGP PUBLIC KEY BLOCK-----/ {
    sub(/^.*"-----BEGIN/, "-----BEGIN")
    print
    in_key = 1
    next
  }
  in_key {
    if (/-----END PGP PUBLIC KEY BLOCK-----/) {
      sub(/";.*/, "")
      print
      exit
    }
    print
  }
' "$key_source" > "$tmp_dir/release-key.asc"

actual_fingerprint=$(
  gpg --batch --with-colons --import-options show-only --import "$tmp_dir/release-key.asc" 2>/dev/null |
    awk -F: '$1 == "fpr" { print $10; exit }'
)
if [[ "$actual_fingerprint" != "$expected_fingerprint" ]]; then
  echo "embedded release key fingerprint mismatch: $actual_fingerprint" >&2
  exit 1
fi

export GNUPGHOME="$tmp_dir/gnupg"
mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --import "$tmp_dir/release-key.asc"

assets=(
  'fluent-devnet|v0.5.7|genesis-v0.5.7.json.gz|91b9a427805d45dd14e46a0cd517bcc85f350fe7dfc38fa96f6ff0ebf5e864da'
  'fluent-testnet|v0.3.4-dev|genesis-v0.3.4-dev.json.gz|8cd30358c5664375e6739bc48302445e7ee10fd0158bedb788505e5c590983bd'
  'fluent-mainnet|v1.0.0|genesis-mainnet-v1.0.0.json.gz|72cb4b3b7b15de952bd1094281a1f2430cb711bc473a0520f92aa3e2b1bdb643'
)

for spec in "${assets[@]}"; do
  IFS='|' read -r network tag name expected_sha256 <<< "$spec"
  grep -Fq "$tag" "$chainspec"
  grep -Fq "$expected_sha256" "$chainspec"

  base_url="https://github.com/fluentlabs-xyz/fluentbase/releases/download/$tag"
  curl --fail --location --silent --show-error --retry 3 --retry-all-errors \
    "$base_url/$name" --output "$tmp_dir/$name"
  curl --fail --location --silent --show-error --retry 3 --retry-all-errors \
    "$base_url/$name.asc" --output "$tmp_dir/$name.asc"

  actual_sha256=$(sha256sum "$tmp_dir/$name" | awk '{print $1}')
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "$network: sha256 mismatch: expected $expected_sha256, got $actual_sha256" >&2
    exit 1
  fi
  gpg --batch --verify "$tmp_dir/$name.asc" "$tmp_dir/$name"
  echo "$network: authenticated $name ($actual_sha256)"
done
