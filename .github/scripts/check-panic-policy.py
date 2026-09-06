#!/usr/bin/env python3
"""Check the documented Cargo panic policies, including profile inheritance."""

from pathlib import Path
import re
import sys
import tomllib


def panic_strategy(profiles, name, visiting=()):
    if name in visiting:
        raise ValueError(f"cyclic profile inheritance: {' -> '.join((*visiting, name))}")
    profile = profiles.get(name, {})
    if "panic" in profile:
        return profile["panic"]
    if "inherits" in profile:
        return panic_strategy(profiles, profile["inherits"], (*visiting, name))
    if name in ("dev", "release"):
        return "unwind"  # Cargo's default for these built-in profiles.
    raise ValueError(f"profile {name!r} is missing or has no inherits setting")


def check_policy(root):
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    profiles = manifest["profile"]
    docs = (root / "docs/05-security-invariants.md").read_text()
    panic_section = docs.split("## 9) Panic policy\n", 1)[1].split("\n## ", 1)[0]
    documented = dict(
        re.findall(r"^\| `(\w+)` \| `(abort|unwind)` \|", panic_section, re.MULTILINE)
    )

    for name in ("release", "maxperf", "reproducible"):
        actual = panic_strategy(profiles, name)
        if name in ("maxperf", "reproducible") and actual != "abort":
            raise ValueError(f"production profile {name!r} must abort, found {actual!r}")
        if documented.get(name) != actual:
            raise ValueError(
                f"profile {name!r}: Cargo uses {actual!r}, "
                f"docs record {documented.get(name)!r}"
            )
        print(f"{name}: panic = {actual} (matches docs)")


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[2]
    try:
        check_policy(root)
    except (KeyError, IndexError, ValueError) as error:
        sys.exit(f"panic policy check failed: {error}")
