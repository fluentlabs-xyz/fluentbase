#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  release-manifest.sh create <manifest> <sbom> <version> <kind> <artifact-dir> [artifact...]
  release-manifest.sh verify <manifest>

Creates and verifies release provenance manifests without network access. The
manifest binds the release commit, lockfile hashes, Rust toolchain, builder
image identity, build features/configuration, runtime/genesis hashes, artifact
hashes, and the SBOM hash.
USAGE
}

json_escape() {
  local s=${1-}
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  printf '%s' "$s"
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

image_digest() {
  local image=${FLUENTBASE_BUILD_DOCKER_IMAGE:-}
  local tag=${FLUENTBASE_BUILD_DOCKER_TAG:-}
  local digest=${FLUENTBASE_BUILD_DOCKER_DIGEST:-}

  if [[ -n "$digest" ]]; then
    printf '%s' "$digest"
    return
  fi

  if [[ -n "$image" && -n "$tag" ]] && command -v docker >/dev/null 2>&1; then
    docker image inspect "${image}:${tag}" \
      --format '{{range .RepoDigests}}{{println .}}{{end}}{{.Id}}' 2>/dev/null \
      | sed '/^$/d' \
      | head -n1
  fi
}

write_lockfiles_json() {
  local first=1 lock
  while IFS= read -r lock; do
    [[ $first -eq 0 ]] && printf ',\n'
    first=0
    printf '      {"path":"%s","sha256":"%s"}' \
      "$(json_escape "$lock")" \
      "$(sha256_file "$lock")"
  done < <(find . -name Cargo.lock -type f | LC_ALL=C sort)
}

write_runtime_json() {
  local first=1 file
  for file in \
    ./crates/genesis/genesis-devnet.json \
    ./crates/genesis/genesis-mainnet.json \
    ./crates/genesis/evm-runtime-permissive.rwasm
  do
    [[ -f "$file" ]] || continue
    [[ $first -eq 0 ]] && printf ',\n'
    first=0
    printf '      {"path":"%s","sha256":"%s"}' \
      "$(json_escape "$file")" \
      "$(sha256_file "$file")"
  done
}

write_artifacts_json() {
  local first=1 file
  for file in "$@"; do
    [[ -f "$file" ]] || continue
    [[ $first -eq 0 ]] && printf ',\n'
    first=0
    printf '      {"path":"%s","sha256":"%s","bytes":%s}' \
      "$(json_escape "$file")" \
      "$(sha256_file "$file")" \
      "$(wc -c < "$file" | tr -d ' ')"
  done
}

create_sbom() {
  local sbom=$1
  local generated_at
  generated_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

  {
    printf '{\n'
    printf '  "sbom_format": "fluentbase-cargo-locked-v1",\n'
    printf '  "generated_at": "%s",\n' "$generated_at"
    printf '  "source": {"repository":"%s","commit":"%s"},\n' \
      "$(json_escape "${GITHUB_REPOSITORY:-unknown}")" \
      "$(json_escape "${GITHUB_SHA:-$(git rev-parse HEAD)}")"
    printf '  "packages": [\n'
    cargo metadata --locked --format-version 1 \
      | python3 -c 'import json,sys
data=json.load(sys.stdin)
pkgs=sorted(data["packages"], key=lambda p: (p["name"], p["version"], p["id"]))
for i,p in enumerate(pkgs):
    prefix="    " if i == 0 else ",\n    "
    print(prefix + json.dumps({
        "name": p["name"],
        "version": p["version"],
        "manifest_path": p["manifest_path"],
        "license": p.get("license"),
        "source": p.get("source"),
    }, sort_keys=True, separators=(",", ":")), end="")
'
    printf '\n  ]\n'
    printf '}\n'
  } > "$sbom"
}

create_manifest() {
  local manifest=$1 sbom=$2 version=$3 kind=$4 artifact_dir=$5
  shift 5

  mkdir -p "$artifact_dir"
  create_sbom "$sbom"

  local generated_at rustc rustup_toolchain digest cargo_config
  generated_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
  rustc=$(rustc -Vv | sed ':a;N;$!ba;s/\n/\\n/g')
  rustup_toolchain=$(rustup show active-toolchain 2>/dev/null | awk '{print $1}' || true)
  digest=$(image_digest || true)
  cargo_config=${BUILD_FEATURES:-${FEATURES:-}}

  {
    printf '{\n'
    printf '  "schema": "https://fluent.xyz/schemas/release-provenance-v1.json",\n'
    printf '  "version": "%s",\n' "$(json_escape "$version")"
    printf '  "kind": "%s",\n' "$(json_escape "$kind")"
    printf '  "generated_at": "%s",\n' "$generated_at"
    printf '  "source": {\n'
    printf '    "repository": "%s",\n' "$(json_escape "${GITHUB_REPOSITORY:-unknown}")"
    printf '    "ref": "%s",\n' "$(json_escape "${GITHUB_REF:-unknown}")"
    printf '    "commit": "%s"\n' "$(json_escape "${GITHUB_SHA:-$(git rev-parse HEAD)}")"
    printf '  },\n'
    printf '  "toolchain": {\n'
    printf '    "rust_toolchain_file": "%s",\n' "$(json_escape "$(tr -d '\n' < rust-toolchain 2>/dev/null || true)")"
    printf '    "rustup_active_toolchain": "%s",\n' "$(json_escape "$rustup_toolchain")"
    printf '    "rustc": "%s"\n' "$(json_escape "$rustc")"
    printf '  },\n'
    printf '  "builder": {\n'
    printf '    "image": "%s",\n' "$(json_escape "${FLUENTBASE_BUILD_DOCKER_IMAGE:-}")"
    printf '    "tag": "%s",\n' "$(json_escape "${FLUENTBASE_BUILD_DOCKER_TAG:-}")"
    printf '    "digest": "%s"\n' "$(json_escape "$digest")"
    printf '  },\n'
    printf '  "build_config": {\n'
    printf '    "target": "%s",\n' "$(json_escape "${BUILD_TARGET:-}")"
    printf '    "profile": "%s",\n' "$(json_escape "${BUILD_PROFILE:-}")"
    printf '    "features": "%s",\n' "$(json_escape "$cargo_config")"
    printf '    "no_default_features": "%s"\n' "$(json_escape "${NO_DEFAULT_FEATURES:-}")"
    printf '  },\n'
    printf '  "lockfiles": [\n'
    write_lockfiles_json
    printf '\n  ],\n'
    printf '  "runtime_and_genesis": [\n'
    write_runtime_json
    printf '\n  ],\n'
    printf '  "artifacts": [\n'
    write_artifacts_json "$@"
    printf '\n  ],\n'
    printf '  "sbom": {"path":"%s","sha256":"%s"}\n' \
      "$(json_escape "$sbom")" \
      "$(sha256_file "$sbom")"
    printf '}\n'
  } > "$manifest"
}

verify_manifest() {
  local manifest=$1
  python3 - "$manifest" <<'PY'
import hashlib
import json
import pathlib
import sys

manifest = pathlib.Path(sys.argv[1])
data = json.loads(manifest.read_text())
errors = []

def check_file(entry, label):
    path = pathlib.Path(entry["path"])
    if not path.exists():
        errors.append(f"{label} missing: {path}")
        return
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != entry["sha256"]:
        errors.append(f"{label} hash mismatch for {path}: {actual} != {entry['sha256']}")

for entry in data.get("lockfiles", []):
    check_file(entry, "lockfile")
for entry in data.get("runtime_and_genesis", []):
    check_file(entry, "runtime/genesis")
for entry in data.get("artifacts", []):
    check_file(entry, "artifact")
check_file(data["sbom"], "sbom")

if not data.get("source", {}).get("commit"):
    errors.append("source commit is missing")
if data.get("builder", {}).get("image") and not data.get("builder", {}).get("digest"):
    errors.append("builder image is set but digest/id is missing")
if not data.get("toolchain", {}).get("rustc"):
    errors.append("rustc version is missing")

if errors:
    for error in errors:
        print(error, file=sys.stderr)
    sys.exit(1)
PY
}

case "${1-}" in
  create)
    [[ $# -ge 6 ]] || { usage; exit 2; }
    shift
    create_manifest "$@"
    ;;
  verify)
    [[ $# -eq 2 ]] || { usage; exit 2; }
    verify_manifest "$2"
    ;;
  *)
    usage
    exit 2
    ;;
esac
