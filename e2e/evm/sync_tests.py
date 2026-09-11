#!/usr/bin/env python3
"""Install the checksum-pinned upstream state fixtures; never modify legacy checkouts."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent


def install(archive, destination, config):
    with archive.open("rb") as source:
        checksum = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
        digest = checksum.hexdigest()
    if digest != config["sha256"]:
        raise ValueError(f"Fixture checksum mismatch: expected {config['sha256']}, got {digest}")

    prefixes = {f"for_{fork.lower()}" for fork in config["forks"]}
    count = 0
    with tarfile.open(archive, "r|gz") as bundle:
        for member in bundle:
            path = PurePosixPath(member.name)
            if path.parts[:2] != ("fixtures", "state_tests"):
                continue
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"Invalid fixture path: {member.name}")
            if len(path.parts) < 4 or path.parts[2] not in prefixes:
                continue
            if not member.isfile() or path.suffix != ".json":
                continue
            target = destination.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            count += 1
    if count == 0:
        raise ValueError("The archive contains no state tests for the configured forks")
    (destination / ".release.json").write_text(json.dumps(config, indent=2) + "\n")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="Use an already downloaded archive (still checksum-verified)")
    args = parser.parse_args()
    config = json.loads((ROOT / "ethereum-tests.json").read_text())
    destination = ROOT / config["directory"]
    if destination.exists():
        marker = destination / ".release.json"
        if marker.is_file() and json.loads(marker.read_text()) == config:
            if all((destination / "state_tests" / f"for_{fork.lower()}").is_dir() for fork in config["forks"]):
                print(f"{config['release']}: already installed at {destination}")
                return
        raise ValueError(f"Unrecognized or incomplete fixture directory: {destination}; move it aside before syncing")

    if destination.parent.is_symlink():
        raise ValueError(f"Refusing to install through a symlink: {destination.parent}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".ethereum-tests-", dir=destination.parent) as work:
        work = Path(work)
        archive = args.archive
        if archive is None:
            archive = work / "fixtures.tar.gz"
            subprocess.run([
                "curl", "--fail", "--location", "--retry", "3", "--silent", "--show-error",
                config["url"], "--output", str(archive),
            ], check=True)
        extracted = work / "extracted"
        extracted.mkdir()
        count = install(archive, extracted, config)
        extracted.rename(destination)
    print(f"{config['release']}: installed {count} state-test files for {', '.join(config['forks'])}")


if __name__ == "__main__":
    main()
