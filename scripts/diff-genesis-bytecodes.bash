#!/usr/bin/env bash
set -euo pipefail

# Compares bytecode from a local genesis JSON file against eth_getCode.
#
# Defaults use the checked-in mainnet genesis and Fluent mainnet RPC. Override
# GENESIS_JSON and RPC_URL to compare another checked-in or generated genesis.
#
# Environment:
#   GENESIS_JSON  local genesis JSON with alloc[].code entries
#   RPC_URL       JSON-RPC endpoint to query (default: Fluent mainnet)
#   BLOCK_TAG     eth_getCode block tag (default: latest)
#   OUTPUT_DIR    where mismatch artifacts are written

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GENESIS_JSON="${GENESIS_JSON:-${REPO_ROOT}/crates/genesis/genesis-mainnet.json}"
RPC_URL="${RPC_URL:-https://rpc.fluent.xyz}"
BLOCK_TAG="${BLOCK_TAG:-latest}"
OUTPUT_DIR="${OUTPUT_DIR:-${REPO_ROOT}/target/genesis-bytecode-diff}"

require_command() {
  local command_name="$1"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 1
  fi
}

require_command python3

if [[ ! -f "$GENESIS_JSON" ]]; then
  echo "error: genesis JSON does not exist: $GENESIS_JSON" >&2
  exit 1
fi

echo "Genesis JSON: $GENESIS_JSON"
echo "RPC URL: $RPC_URL"
echo "Block tag: $BLOCK_TAG"
echo "Output dir: $OUTPUT_DIR"

mkdir -p "$OUTPUT_DIR"

GENESIS_JSON="$GENESIS_JSON" \
RPC_URL="$RPC_URL" \
BLOCK_TAG="$BLOCK_TAG" \
OUTPUT_DIR="$OUTPUT_DIR" \
python3 - <<'PY'
import json
import os
import re
import urllib.error
import urllib.request
from pathlib import Path

genesis_json = Path(os.environ["GENESIS_JSON"])
rpc_url = os.environ["RPC_URL"]
block_tag = os.environ["BLOCK_TAG"]
output_dir = Path(os.environ["OUTPUT_DIR"])


def rpc(method: str, params):
    payload = json.dumps(
        {"jsonrpc": "2.0", "method": method, "params": params, "id": 1}
    ).encode("utf-8")
    request = urllib.request.Request(
        rpc_url,
        data=payload,
        headers={"content-type": "application/json", "user-agent": "curl/8.0"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            data = json.loads(response.read().decode("utf-8"))
    except urllib.error.URLError as exc:
        raise SystemExit(f"error: RPC request failed: {exc}") from exc
    if "error" in data:
        raise SystemExit(f"error: RPC returned error for {method}: {data['error']}")
    return data.get("result")


def normalize_hex(value: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"expected hex string, got {type(value).__name__}")
    value = value.lower()
    if value.startswith("0x"):
        value = value[2:]
    if len(value) % 2:
        value = "0" + value
    return value


def write_hex(path: Path, hex_value: str) -> None:
    lines = [hex_value[index : index + 64] for index in range(0, len(hex_value), 64)]
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def safe_address(address: str) -> str:
    return re.sub(r"[^a-fA-F0-9x]+", "-", address.lower())


def first_diff(local: bytes, remote: bytes):
    shared_len = min(len(local), len(remote))
    for index in range(shared_len):
        if local[index] != remote[index]:
            return index, local[index], remote[index]
    if len(local) != len(remote):
        return shared_len, None, None
    return None


with genesis_json.open(encoding="utf-8") as genesis_file:
    genesis = json.load(genesis_file)

alloc = genesis.get("alloc")
if not isinstance(alloc, dict):
    raise SystemExit(f"error: genesis JSON does not contain an alloc object: {genesis_json}")

contracts = []
for address, account in alloc.items():
    if not isinstance(account, dict):
        continue
    code = account.get("code")
    if code is None:
        continue
    contracts.append((address.lower(), normalize_hex(code)))

if not contracts:
    raise SystemExit(f"error: no alloc[].code entries found in {genesis_json}")

print(f"Contracts: {len(contracts)}")

summary = {"ok": 0, "diff": 0, "missing": 0}
rows = []
for address, local_hex in sorted(contracts, key=lambda item: item[0]):
    local_bytes = bytes.fromhex(local_hex)
    remote_hex = normalize_hex(rpc("eth_getCode", [address, block_tag]))
    remote_bytes = bytes.fromhex(remote_hex)

    base = output_dir / safe_address(address)
    if remote_hex == "":
        summary["missing"] += 1
        print(f"MISSING {address} local_len={len(local_bytes)} remote_len=0")
        rows.append([address, "MISSING", str(len(local_bytes)), "0", ""])
        write_hex(base.with_suffix(".local.hex"), local_hex)
        base.with_suffix(".local.bin").write_bytes(local_bytes)
        continue

    if local_hex == remote_hex:
        summary["ok"] += 1
        print(f"OK      {address} len={len(local_bytes)}")
        rows.append([address, "OK", str(len(local_bytes)), str(len(remote_bytes)), ""])
        continue

    summary["diff"] += 1
    diff = first_diff(local_bytes, remote_bytes)
    if diff is None:
        diff_text = "unknown"
    else:
        offset, local_byte, remote_byte = diff
        if local_byte is None or remote_byte is None:
            diff_text = f"first_diff_offset={offset} length-only"
        else:
            diff_text = (
                f"first_diff_offset={offset} "
                f"local=0x{local_byte:02x} remote=0x{remote_byte:02x}"
            )
    print(
        f"DIFF    {address} "
        f"local_len={len(local_bytes)} remote_len={len(remote_bytes)} {diff_text}"
    )
    rows.append([address, "DIFF", str(len(local_bytes)), str(len(remote_bytes)), diff_text])
    write_hex(base.with_suffix(".local.hex"), local_hex)
    write_hex(base.with_suffix(".remote.hex"), remote_hex)
    base.with_suffix(".local.bin").write_bytes(local_bytes)
    base.with_suffix(".remote.bin").write_bytes(remote_bytes)

summary_path = output_dir / "summary.tsv"
summary_path.write_text(
    "address\tstatus\tlocal_len\tremote_len\tdetail\n"
    + "\n".join("\t".join(row) for row in rows)
    + "\n",
    encoding="utf-8",
)

print(
    "Summary: "
    f"ok={summary['ok']} diff={summary['diff']} missing={summary['missing']} "
    f"artifacts={output_dir} summary={summary_path}"
)
PY
