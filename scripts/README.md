# Scripts

Developer and CI helper scripts for the repository:

- [`local-ci.sh`](local-ci.sh) — runs the full Linux PR check suite locally (`fmt`, changelog, strings, no-sites, spelling, `affected-crates`, clippy, tests, and tester build). Pass `--quick` for fast non-Cargo checks or `--no-build` to skip the desktop build.
- [`changelog-draft.sh`](changelog-draft.sh) — prints commits since the last stable release for `desktop` or `phone` when curating `CHANGELOG.md`.
- [`ci/`](ci/) — individual check and release scripts shared between `local-ci.sh` and `.github/workflows/`:
  - `affected-crates.py` — computes affected crates from `cargo metadata` and validates workflow path filters (`--self-test`).
  - `check-structure.py` — verifies top-level directory cleanliness and ensures every crate/workspace is documented in `ARCHITECTURE.md`.
  - `strings-check.py` / `strings-baseline.txt` — verifies all UI strings go through `i18n/`.
  - `no-site-names.sh` — enforces the no-download-sites policy across file contents and filenames.
  - `one-spelling.sh` — enforces consistent `Anibeam` casing.
  - `changelog-newest.sh` — ensures PRs do not edit shipped changelog headings.
  - `release-name.sh` — derives release titles from `CHANGELOG.md`.
  - `ios-automerge.sh` — squash-merges eligible PRs once checks pass.
