# AGENTS.md

Instructions for AI coding agents working in this repository. Start by reading the project's human-first docs so your work stays aligned with everyone else:

- [ARCHITECTURE.md](ARCHITECTURE.md) — repo layout, crate dependency layers, routing table, and cross-cutting engineering invariants.
- [CONTRIBUTING.md](CONTRIBUTING.md) — issue/branch/commit naming, PR rules, i18n catalog rules, no-sites policy, and CI checks.
- [CONTEXT.md](CONTEXT.md) — canonical domain glossary (use these exact terms; avoid the listed synonyms).
- [docs/build-from-source.md](docs/build-from-source.md) — toolchain and native dependency setup.
- [docs/safety-audit.md](docs/safety-audit.md) — security surface index (start here if auditing installation/runtime safety).

## Git workflow

- **Never commit directly on `main`.** Work on a focused branch named `<type>/<issue>-<slug>` and squash-merge via PR (`Closes #<issue>`).
- **Check before opening a PR:** Run `.github/local-ci.sh` locally (or `--no-build` / `--quick` while iterating). Never leave `main` broken.
- **Never create releases or `android-v*` pre-releases by hand** for test builds; see [docs/releasing.md](docs/releasing.md).

## Build and test commands

Cargo builds the workspace; there is no Node or Bun build step.

```bash
# Iterate on only the crate you changed:
cargo test -p anibeam-library
cargo clippy -p anibeam-library --all-targets -- -D warnings

# Full workspace checks (run before opening a PR):
.github/local-ci.sh --no-build
cargo fmt --all --check

# Desktop shell (needs Qt 6.11+, qmake6, MpvQt, libmpv, lld):
cargo build --release -p anibeam

# Terminal CLI over the same core:
cargo build --release -p anibeam-cli
```

### Fast QML iteration without rebuilding Rust

A tester build with `dev-qml` loads QML from disk when `ANIBEAM_QML_DIR` is set, so QML-only edits need no Cargo rebuild:

```bash
cargo build --profile tester -p anibeam --features dev-qml
ANIBEAM_QML_DIR=apps/linux/qml \
ANIBEAM_THEMES_DIR=apps/linux/themes \
ANIBEAM_MPV_CONF=apps/linux/mpv.conf \
  target/tester/anibeam --root /tmp/anibeam-dev
```

Always develop, test, and capture screenshots against an isolated `--root <dir>` sandbox so you never touch a user's real XDG data or existing qBittorrent setup.

### Driving the UI programmatically

- **Desktop:** Use `anibeam ctl` instead of simulating raw mouse clicks (`anibeam ctl help`). `anibeam ctl open --root <dir> --offscreen` launches a headless sandbox window, followed by `go <page>`, `tree`, `click <words|objectName|x,y>`, `type`, `key`, `shot <file.png>`, `where`, `eval <js>`, and `quit`. See `apps/linux/README.md` for `scripts/shoot.sh`, `scripts/drive.sh`, and `scripts/longshot.py`.
- **Phone:** Use `apps/phone/scripts/ctl.sh` (`ctl.sh help`, `tree`, `go`, `click`, `type`, `key`, `shot`, `wait`, `perf`) over `adb` (`$ANDROID_SERIAL`). See `apps/phone/README.md`.

## Workflow guides for repository automation

- Issue tracking and project board state: [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)
- Incoming issue triage: [docs/agents/triage.md](docs/agents/triage.md)
- Backlog review: [docs/agents/backlog-review.md](docs/agents/backlog-review.md)
- Domain glossary & ADR usage: [docs/agents/domain.md](docs/agents/domain.md)
