#!/usr/bin/env python3
"""Verify the repository stays organized and ARCHITECTURE.md stays current.

Checks:
  1. Every tracked top-level directory belongs to the intentional root layout
     (new Rust crates go in crates/, apps in apps/, relays in relay/, scripts in scripts/).
  2. Every visible top-level directory appears at the top level of the repository map tree in
     ARCHITECTURE.md.
  3. Every Cargo workspace member (plus standalone workspaces apps/phone and relay/worker) is
     documented by full path in ARCHITECTURE.md.
  4. Every Cargo.toml directly under crates/* is listed in root Cargo.toml workspace.members.
"""

import os
import pathlib
import re
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent

ALLOWED_ROOT_DIRS = {
    ".cargo",
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

TREE_LINE_RE = re.compile(r"^((?:│   |    )*)(?:├── |└── )([A-Za-z0-9._-]+)/")
BACKTICK_PATH_RE = re.compile(r"`([A-Za-z0-9._-]+(?:/[A-Za-z0-9._-]+)+)/?`")


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


def parse_architecture_paths(arch: str) -> tuple[set[str], set[str]]:
    """Return (top_level_tree_dirs, all_full_paths) documented in ARCHITECTURE.md."""
    top_level: set[str] = set()
    full_paths: set[str] = set()
    stack: list[str] = []
    in_fence = False

    for line in arch.splitlines():
        stripped = line.strip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            stack.clear()
            continue

        if in_fence:
            m = TREE_LINE_RE.match(line)
            if m:
                depth = len(m.group(1)) // 4
                name = m.group(2)
                stack = stack[:depth]
                stack.append(name)
                rel = "/".join(stack)
                full_paths.add(rel)
                if depth == 0:
                    top_level.add(name)
            elif not stripped:
                stack.clear()

        for m in BACKTICK_PATH_RE.finditer(line):
            full_paths.add(m.group(1))

    return top_level, full_paths


def main() -> int:
    problems: list[str] = []

    arch_path = ROOT / "ARCHITECTURE.md"
    if not arch_path.is_file():
        print("check-structure: ARCHITECTURE.md is missing at the repository root", file=sys.stderr)
        return 1
    arch = arch_path.read_text(encoding="utf-8")
    top_level_dirs, documented_paths = parse_architecture_paths(arch)

    # 1. No unexpected top-level directories
    actual_dirs = tracked_root_dirs()
    unexpected = sorted(actual_dirs - ALLOWED_ROOT_DIRS)
    for d in unexpected:
        problems.append(
            f"unexpected top-level directory `{d}/`: place library crates in `crates/`, "
            f"apps in `apps/`, relays in `relay/`, scripts in `scripts/`, or add `{d}` to "
            "ALLOWED_ROOT_DIRS in scripts/ci/check-structure.py and document it in ARCHITECTURE.md"
        )

    # 2. Every visible root directory appears at top level in ARCHITECTURE.md's repository map
    for d in sorted(actual_dirs):
        if d.startswith("."):
            continue
        if d not in top_level_dirs:
            problems.append(
                f"top-level directory `{d}/` is not listed at the top level of the repository map in ARCHITECTURE.md"
            )

    # 3. Every Cargo workspace member and standalone workspace appears by full path in ARCHITECTURE.md
    for member in workspace_members() + STANDALONE_WORKSPACES:
        if member not in documented_paths:
            problems.append(
                f"workspace member `{member}` is not documented by full path in ARCHITECTURE.md"
            )

    # 4. Every Cargo.toml directly under crates/* is a workspace member
    members = set(workspace_members())
    crates_dir = ROOT / "crates"
    if crates_dir.is_dir():
        for entry in sorted(os.listdir(crates_dir)):
            sub = crates_dir / entry
            if (sub / "Cargo.toml").is_file():
                rel = f"crates/{entry}"
                if rel not in members:
                    problems.append(
                        f"`{rel}` has a Cargo.toml but is not listed in root Cargo.toml workspace.members"
                    )

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
