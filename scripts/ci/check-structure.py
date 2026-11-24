#!/usr/bin/env python3
"""Verify the repository stays organized and ARCHITECTURE.md stays current.

Checks:
  1. Every tracked top-level directory belongs to the intentional root layout
     (new Rust crates go in crates/, apps in apps/, relays in relay/, scripts in scripts/).
  2. Every visible top-level directory and every Cargo workspace member (plus standalone
     workspaces apps/phone and relay/worker) is documented in ARCHITECTURE.md.
"""

import os
import pathlib
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent

ALLOWED_ROOT_DIRS = {
    ".config",
    ".github",
    "Casks",
    "apps",
    "assets",
    "crates",
    "docs",
    "i18n",
    "relay",
    "scripts",
    "site",
}

STANDALONE_WORKSPACES = [
    "apps/phone",
    "relay/worker",
]


def tracked_root_dirs() -> set[str]:
    out = subprocess.check_output(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        text=True,
    )
    dirs = set()
    for path in out.split("\0"):
        if "/" in path:
            dirs.add(path.split("/", 1)[0])
    return dirs


def workspace_members() -> list[str]:
    with open(ROOT / "Cargo.toml", "rb") as fh:
        manifest = tomllib.load(fh)
    return list(manifest.get("workspace", {}).get("members", []))


def main() -> int:
    problems: list[str] = []

    arch_path = ROOT / "ARCHITECTURE.md"
    if not arch_path.is_file():
        print("check-structure: ARCHITECTURE.md is missing at the repository root", file=sys.stderr)
        return 1
    arch = arch_path.read_text(encoding="utf-8")

    # 1. No unexpected top-level directories
    actual_dirs = tracked_root_dirs()
    unexpected = sorted(actual_dirs - ALLOWED_ROOT_DIRS)
    for d in unexpected:
        problems.append(
            f"unexpected top-level directory `{d}/`: place library crates in `crates/`, "
            f"apps in `apps/`, relays in `relay/`, scripts in `scripts/`, or add `{d}` to "
            "ALLOWED_ROOT_DIRS in scripts/ci/check-structure.py and document it in ARCHITECTURE.md"
        )

    # 2. Every visible root directory appears in ARCHITECTURE.md
    for d in sorted(actual_dirs):
        if d.startswith("."):
            continue
        if f"{d}/" not in arch:
            problems.append(f"top-level directory `{d}/` is not documented in ARCHITECTURE.md")

    # 3. Every Cargo workspace member and standalone workspace appears in ARCHITECTURE.md
    for member in workspace_members() + STANDALONE_WORKSPACES:
        leaf = member.split("/")[-1]
        if f"{member}/" not in arch and f"{member}`" not in arch and f"├── {leaf}/" not in arch and f"└── {leaf}/" not in arch:
            problems.append(f"workspace member `{member}` is not documented in ARCHITECTURE.md")

    # 4. Every Cargo.toml directly under crates/* is a workspace member
    members = set(workspace_members())
    crates_dir = ROOT / "crates"
    if crates_dir.is_dir():
        for entry in sorted(os.listdir(crates_dir)):
            sub = crates_dir / entry
            if (sub / "Cargo.toml").is_file():
                rel = f"crates/{entry}"
                if rel not in members:
                    problems.append(f"`{rel}` has a Cargo.toml but is not listed in root Cargo.toml workspace.members")

    if problems:
        for p in problems:
            print(f"check-structure: {p}", file=sys.stderr)
        return 1

    print(
        f"structure ok: {len(actual_dirs)} root dirs, "
        f"{len(workspace_members())} workspace members + {len(STANDALONE_WORKSPACES)} standalone workspaces documented"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
