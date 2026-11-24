# Contributing

Welcome! New to the codebase? Start with [ARCHITECTURE.md](ARCHITECTURE.md) for a map of where everything lives and [docs/build-from-source.md](docs/build-from-source.md) to get a local build running.

One naming scheme runs from issue to branch, commits, PR title, and the squash commit on `main`.

## The flow

1. Start with a GitHub issue, except for typo-sized edits (see "The PR body"). State the user-visible outcome in the title and what "done" looks like in the body. Before moving cards on the project board, ask a maintainer for access and run `gh auth refresh -s project`. Report security issues privately per [SECURITY.md](SECURITY.md).
2. Branch off `main` as `<type>/<issue>-<slug>`.
3. Commit as you go with `<type>(<scope>): <outcome>`.
4. Try the branch live in an isolated `--root` sandbox (see below).
5. Open a PR whose title follows the same convention and whose body ends with `Closes #<issue>`.
6. Address review feedback as new commits (do not amend or force-push a pushed branch) and make sure all checks pass.
7. Squash merge onto `main` and delete the branch.

## Types

| Type | Use it for |
| --- | --- |
| `feat` | something new the app can do |
| `fix` | something broken now works |
| `perf` | same behaviour, less CPU, memory or frame time |
| `refactor` | code moves or changes shape, behaviour stays |
| `docs` | README, ARCHITECTURE, CONTEXT, CONTRIBUTING, `docs/` |
| `test` | tests only |
| `chore` | dependencies, tooling, cleanup |
| `ci` | `.github/workflows` |

## Scopes

Name the crate or area the change belongs to: `core`, `library`, `metadata`, `trackers`, `downloads`, `playback`, `releases`, `host`, `net`, `foundation`, `contract`, `shell` (for `apps/desktop`), `cli` (for `apps/cli`), or `relay`. Desktop UI changes scoped to one screen may name it directly: `home`, `library`, `series`, `feed`, `watching`, `downloads`, `metadata`, `tokens`, `settings`, `player`.

For `apps/phone` (Android and iOS):
- `android`: Android-only files (`android/`, `src/platform/android.rs`)
- `ios`: iOS-only files (`ios/`, `src/platform/ios.rs`, `*ios*` scripts)
- `phone core`: shared mobile code built into both platforms

For documentation changes, name the document (`readme`, `architecture`, `contributing`, `context`). Outside crates and docs, use `packaging`, `site`, or `ci`. Comma-separate two scopes when genuinely spanning both (`fix(core,shell): ...`); more than two usually means the PR should be split.

## Branch names

```text
<type>/<issue>-<slug>
```

Examples: `feat/69-downloads-redesign`, `perf/64-shared-cargo-target`, `docs/contributing-naming` (for a `no issue` typo PR).

## Trying a branch live

Always run development builds against a sandbox `--root` directory so they stay isolated from any installed copy and never touch your real library database, secrets, or torrent client:

```bash
cargo build --release -p anibeam
ANIBEAM_THEMES_DIR=apps/desktop/themes ANIBEAM_MPV_CONF=apps/desktop/mpv.conf \
  target/release/anibeam --root /tmp/anibeam-dev
```

`--root` isolates the database, config, cache, secrets, single-instance bus names, and managed `anirss`/qBittorrent state inside one folder. For headless captures and scripted UI checks, see `apps/desktop/README.md` (`apps/desktop/scripts/shoot.sh`, `apps/desktop/scripts/drive.sh`, and `anibeam ctl`).

## Commit and PR titles

```text
<type>(<scope>): <what is different now>
```

Describe the outcome a user would notice, lowercase after the colon, no trailing period.

Good examples:
- `fix(shell): the wheel scrolls like a browser`
- `feat(downloads): a provider switch on the search bar`

Avoid:
- `Fix bug`, `shell: update WheelStep.qml`, or raw merge commits

## The PR body

Include:
- What changed, in a few lines.
- Why, referencing the issue.
- How you verified it (screenshot or recording for visible changes, test output otherwise).
- `Closes #<issue>` on its own line.

Every PR closes an open issue (`Closes`, `Fixes`, or `Resolves`), enforced by the `PR issue` check. Bare mentions (`Part of #N`, `See #N`) or links to already-closed issues do not satisfy the check: file a sub-issue or follow-up issue instead. Maintainers may apply the `no issue` label for typo-sized PRs; do not add `no issue` to your own PR to get past the check.

## Patch notes

Each app maintains its own curated `CHANGELOG.md` (`apps/desktop/CHANGELOG.md` for desktop, `apps/phone/CHANGELOG.md` for phone). Both apps embed it for "What's new", and `anibeam.app/whats-new/` builds from the same files.

- Regular PRs do not touch `CHANGELOG.md`. Per-PR edits cause merge conflicts and noisy dumps, and GitHub Releases already lists every PR via `--generate-notes`.
- Maintainers curate entries before shipping using `scripts/changelog-draft.sh desktop|phone`: a short, warm 1 to 2 sentence intro, 3 to 8 user-focused highlight bullets (`- **Feature.** Description.`), an optional `### Heads up`, and a concise `### Fixes & polish` summary. Use plain English, no issue numbers, and no em or en dashes.
- Headings match the app version: `## 2.5.0 · 2026-11-21` or `## 2.5.0 · Short title · 2026-11-21` (title max 40 chars). Bumping the version adds a new heading, and a new heading bumps the version in the same PR.

CI checks both changelogs via `cargo test -p anibeam-core --test changelog` and `scripts/ci/changelog-newest.sh`.

## Words (i18n)

Every user-visible string lives in the Fluent catalog under `i18n/`: `i18n/<lang>/core.ftl` for core messages, `desktop.ftl` for desktop, and `android.ftl` for phone. English (`i18n/en/`) is the source; missing IDs in other languages fall back to English.

- Use stable, descriptive message IDs (`player-skip-intro`, `downloads-empty`), prefixed by the file or feature name, with Fluent plurals and arguments rather than concatenating strings in code.
- Look up messages via `anibeam_core::i18n::tr(...)` in Rust, `words.t(...)` or component `msg:` props in QML (never name a QML element `id: words`), and `Tr` in Slint.
- Add new English IDs in the same PR; translations may land in the same PR or follow later.
- Check layout overflow across languages after copy or layout changes with `apps/desktop/scripts/layout-check.py --root <sandbox>`.

CI runs `scripts/ci/strings-check.py` to reject raw user-facing UI strings and missing English IDs. Add `// i18n: none` to non-text literals (format strings, etc.). If your PR removes legacy strings listed in `scripts/ci/strings-baseline.txt`, update the baseline with `scripts/ci/strings-check.py --update`.

## Sources

Anibeam ships no download sources and accepts no PR or issue that names or links a download site or source list. CI enforces this across file contents and filenames via `scripts/ci/no-site-names.sh`.

- The core implements generic endpoint kinds (RSS, Torznab, JSON feeds) named for their input shape, never for a site. PRs adding a new generic kind are welcome (open an issue first).
- Actual sources live in user-attached signed endpoint repos; see [anibeam-endpoints-template](https://github.com/marcusbandit/anibeam-endpoints-template) for the schema and signing workflow.
- Anibeam never bundles, starts, or special-cases third-party proxy services for a specific site.

## Before you open a PR

- `scripts/local-ci.sh` passes (fmt, changelog heading, strings, no-site-names, spelling, repo structure, clippy, tests, and tester build).
- Visible UI changes include a capture taken against a `--root` sandbox.
- New domain concepts are defined in [CONTEXT.md](CONTEXT.md).
- No download site names appear anywhere in the diff.
- New user-facing text lives in `i18n/en/`.

## Checks on GitHub

Pull requests run fast affected-crate checks (`scripts/ci/affected-crates.py`), fmt, strings, no-sites, repo structure, PR checks, and mobile checks when phone or core code changes. Installing [cargo-nextest](https://nexte.st) (`cargo install --locked cargo-nextest`) speeds up `scripts/local-ci.sh` to match CI.

When a PR is ready for full cross-platform verification (Linux AppImage and distro smoke tests, Windows, macOS, Arch tarball, iOS IPA), maintainers apply the `full ci` label. For release promotion, hotfixes, and automerge details, see [docs/releasing.md](docs/releasing.md).
