# Architecture

Anibeam is a local anime library, video player, and tracker built around one Rust core and thin per-platform shells. All business rules (scanning, matching, tracker sync, downloads, playback rules, watch-together rooms, and Tailscale streaming) live in Rust library crates. Shells handle windows, UI, input, OS integration, and the video surface.

For domain vocabulary shared across code and docs, see [CONTEXT.md](CONTEXT.md).

## Repository map

```text
anibeam/
├── apps/                    # User-facing applications
│   ├── desktop/             #   Desktop app: Qt 6 + QML (Linux, macOS, Windows)
│   ├── phone/               #   Mobile app: Slint (Android & iOS) + companion crate
│   └── cli/                 #   Terminal CLI over anibeam-core
├── crates/                  # 14 layered Rust core crates (listed below)
├── relay/                   # Watch Together relays and the dev Worker
│   ├── server/              #   Self-hosted native Rust relay (anibeam-relay)
│   ├── worker/              #   Cloudflare Worker WASM relay (rooms.anibeam.app)
│   └── dev/                 #   Cloudflare Worker for developer endpoints (dev.anibeam.app)
├── i18n/                    # Fluent (.ftl) catalog for all 13 languages
├── assets/                  # Canonical brand logos & icon generators
├── site/                    # anibeam.app website, wiki, and installer scripts
├── scripts/                 # Developer & CI scripts (`local-ci.sh`, `ci/*`)
├── docs/                    # Living guides, ADRs, safety audit, and README media
├── Casks/                   # Homebrew cask (pinned at root for `brew tap`)
├── build.sh                 # One-command desktop build script
└── Cargo.toml               # Root Rust workspace manifest
```

### UI directory layout

- Desktop QML lives in `apps/desktop/qml/`, with `Main.qml` at the root and subfolders `shell/` (frame, nav, rail, drawers, onboarding), `ui/` (tokens, i18n, shared primitives), `pages/` (top-level rail pages), `series/`, `player/`, `room/`, `downloads/`, `settings/`, and `dev/` (drive, ctl, and HUD tools).
- Phone Slint lives in `apps/phone/ui/`, with `app.slint` at the root and subfolders `core/` (theme, i18n, data, widgets, sheets), `pages/` (main tabs and player), `show/` (series and show detail sheets and sections), and `more/` (settings, stats, and backup sub-views), plus `fonts/` and `icons/`.

## Crate layers

The Rust workspace is split into focused crates arranged in strict dependency layers so you can build and test one area (`cargo test -p anibeam-library`) without rebuilding the whole app:

```text
  Apps & Services     apps/desktop (anibeam) · apps/phone · apps/cli · relay/server · relay/worker · relay/dev
                             │                                           │
  Core Facade         core (anibeam-core: Core router, launch, update, changelog, stats, agent, tests/)
                             │
  Domain Crates       host ──┬── playback ──┬── downloads ──┬── trackers ──┬── metadata   releases
                             │              │               │              │       │          │
                             └──────────────┴───────────────┴──────────────┴───► library ◄────┘
                                                                                   │
  Platform            net (rate-limited APIs, DoH, image cache) ──────────► foundation (SQLite, bus, jobs, secrets, prefs)
                                                                                   │
  Pure Leaves         contract (Calls, Replies, Events, Errors, i18n) · host-api · room · recs
```

| Crate | Package | Responsibility |
| --- | --- | --- |
| `crates/contract/` | `anibeam-contract` | Public shell/core boundary: `Call`, `Reply`, `Event`, `Error`, records, and the `i18n/` Fluent catalog loader. |
| `crates/host-api/` | `anibeam-host-api` | Wire records and Ed25519 request-proof auth for Tailscale host and phone communication. |
| `crates/room/` | `anibeam-room` | Pure, I/O-free Watch Together room protocol, state machine, rate limits, and join HTML page. Shared by desktop, phone, native relay, and Cloudflare Worker. |
| `crates/recs/` | `anibeam-recs` | Pure recommendation scoring and ranking algorithm. |
| `crates/foundation/` | `anibeam-foundation` | Shared infrastructure: SQLite store and migrations (`src/store/sql/`), event bus, job tracker, paths, prefs, profiles, OS keyring and file secrets, secret redaction, and `Services`. |
| `crates/net/` | `anibeam-net` | Rate-limited HTTP clients (`src/limiter.rs`) for AniList, MAL, Jikan, TVmaze, AniSkip, ani.zip, IMDb, and Wikidata, plus DoH, domain fronting, and image caching. |
| `crates/library/` | `anibeam-library` | Sources, folder walk, filesystem watcher, episode filename classifier, quick fixes, history, delete and trash, and library read models (cards, details, snapshot). |
| `crates/releases/` | `anibeam-releases` | Release title parsing, RSS/Torznab/JSON feed readers, best-fit ranking, refinement grid, rules, tracker scrape stats, and signed endpoint repos. |
| `crates/metadata/` | `anibeam-metadata` | Auto-matching, metadata fetching, franchise crawl and closure (`src/franchise/`), Airing now, calendar feed, and Recommendations wiring (`src/recommended/`). |
| `crates/trackers/` | `anibeam-trackers` | AniList and MyAnimeList OAuth, list cache, two-way writes, local lists, Watching, move-up, On hold, and Planning lists. |
| `crates/downloads/` | `anibeam-downloads` | Built-in native downloads engine, `anirss` sidecar, qBittorrent client, rolling downloads, episode watch, pruning, and phone torrent engine (`src/phone/` behind `phone` feature). |
| `crates/playback/` | `anibeam-playback` | Session rules (30s view, 85% mark, completion), track memory, skip windows, mpv options, Discord Rich Presence (`src/presence/`), Chromecast/DLNA casting (`src/cast/`, `cast` feature), and Watch Together room client (`src/room/`). |
| `crates/host/` | `anibeam-host` | Tailscale library host server, peer trust, ffmpeg transcoding, streaming, and JSON library export/import (`src/transfer/`). |
| `crates/core/` | `anibeam-core` | Top-level facade assembling all crates into `Core` and its call router, plus launch, auto-updater, changelog parser, schedule agent, watching stats, UniFFI exports, and end-to-end integration tests (`crates/core/tests/`). |

## Standalone Cargo workspaces

Three directories intentionally define their own `[workspace]` and `Cargo.lock` so `cargo test --workspace` at the repo root stays fast and does not require mobile NDKs or WASM toolchains:

- `apps/phone/` (members `.` and `companion/`) builds the Android native library and iOS executable on top of `anibeam-core` with `--features phone,cast`.
- `relay/worker/` builds the `wasm32-unknown-unknown` Cloudflare Worker on top of `anibeam-room`.
- `relay/dev/` builds the `wasm32-unknown-unknown` Cloudflare Worker for the developer endpoints at `dev.anibeam.app` (the usage log's collection point), with no workspace crates.

## Where to make changes

| Task | Start here |
| --- | --- |
| A desktop screen, control, or theme | `apps/desktop/qml/`, `apps/desktop/themes/`, and `apps/desktop/src/bridge/` |
| Desktop video player or C++/Obj-C++ glue | `apps/desktop/cpp/` (`videoitem.*`, `nowplaying.mm`) and `apps/desktop/mpv.conf` |
| Desktop installers / OS packaging | `apps/desktop/packaging/{linux,macos,windows,aur,brew}/` and `Casks/anibeam.rb` |
| A mobile screen, Android Java, or iOS setup | `apps/phone/ui/`, `apps/phone/src/`, `apps/phone/android/`, `apps/phone/ios/` |
| Phone Tailscale companion client | `apps/phone/companion/src/` and `crates/host-api/src/` |
| Any user-visible text or translation | `i18n/en/{core,desktop,android}.ftl` (then other `i18n/<lang>/` folders) |
| A new core Call, Reply, or Event | `crates/contract/src/{calls,records,events}.rs`, routed in `crates/core/src/core.rs` |
| SQLite schema or a new migration | `crates/foundation/src/store/sql/` and `crates/foundation/src/store/migrations.rs` |
| Episode filename parsing / quick fixes | `crates/library/src/classifier.rs`, `crates/library/src/readings.rs`, `crates/library/src/fixes.rs` |
| External API client or rate limit | `crates/net/src/` (always through `crates/net/src/limiter.rs`) |
| Auto-matching or franchise relation graph | `crates/metadata/src/automatch.rs` and `crates/metadata/src/franchise/` |
| AniList / MyAnimeList sync or OAuth | `crates/trackers/src/` |
| Release parsing, feeds, or endpoint repos | `crates/releases/src/` |
| Torrent client, rolling downloads, or phone torrents | `crates/downloads/src/` |
| Playback rules, skip windows, Cast, Discord, or Rooms | `crates/playback/src/` |
| Tailscale host server or backup export/import | `crates/host/src/` |
| Watch Together relay servers | `relay/server/` (self-hosted) and `relay/worker/` (Cloudflare Worker) |
| Developer endpoints (`dev.anibeam.app`, usage log sends) | `relay/dev/` and [docs/agents/dev-endpoint.md](docs/agents/dev-endpoint.md) |
| Website, wiki, or What's new pages | `site/` (`apps/desktop/CHANGELOG.md` and `apps/phone/CHANGELOG.md` feed What's new) |
| Security and trust surface audit | [docs/safety-audit.md](docs/safety-audit.md) |

## Cross-cutting invariants

Every contributor (human or AI) should preserve these rules:

- Every outbound provider call (AniList, MAL, Jikan, TVmaze, AniSkip, and the rest of `crates/net/src/`) goes through the shared rate limiter in `crates/net/src/limiter.rs`. Never bypass it.
- Keep the activity log signal-only: record state changes, never per-asset, per-candidate, or per-browse chatter.
- Preserve franchise closure boundaries: `crates/metadata/src/franchise/closure.rs` draws `CHARACTER` and `OTHER` edges but never walks across them, preventing crossover cameos from merging unrelated franchises.
- Keep provider score scales on write: store `average_score` as received from the provider and normalize it at read time.
- Never poll the library on a timer: rely on the filesystem watcher plus a catch-up scan at launch, with no periodic rescan intervals.
- Keep playback in one window and playing when hidden: never open a second window or external mpv process, and never auto-pause when the window is hidden.
- Keep desktop behavior self-contained with no compositor-specific coupling (such as `hyprctl` or Hyprland window rules).
- Keep desktop scrolling native: every `Flickable` uses `apps/desktop/qml/ui/WheelStep.qml` wheel handling (100 px per notch glided to one target).
- Ship no download sources: nothing in this repo names or points at a download site, in code, defaults, tests, fixtures, or docs. The core defines generic endpoint kinds (RSS, Torznab, JSON) only, and users add signed endpoint repos themselves. Enforced by `scripts/ci/no-site-names.sh`.
- Keep all UI copy in `i18n/`: shells never hardcode user-facing English strings and look up Fluent message IDs instead. Enforced by `scripts/ci/strings-check.py`.
- Keep patch notes curated for people: regular PRs do not edit `CHANGELOG.md`. Curate `apps/desktop/CHANGELOG.md` and `apps/phone/CHANGELOG.md` before releases; `site/whats-new.py` builds `anibeam.app/whats-new/` directly from them.
