# Architecture

Anibeam is a local anime library, video player, and tracker built around **one Rust core** and **thin per-platform shells**. All business rules—scanning, matching, tracker sync, downloads, playback rules, watch-together rooms, and Tailscale streaming—live in Rust library crates. Shells handle windows, UI, input, OS integration, and the video surface.

For domain vocabulary shared across code and docs, see [CONTEXT.md](CONTEXT.md).

## Repository map

```text
anibeam/
├── apps/                    # User-facing applications & servers
│   ├── linux/               #   Desktop app: Qt 6 + QML (Linux, macOS, Windows)
│   ├── phone/               #   Mobile app: Slint (Android & iOS) + companion crate
│   ├── cli/                 #   Terminal CLI over anibeam-core
│   └── relay/               #   Self-hosted Watch Together relay server
├── crates/                  # 14 layered Rust core crates (listed below)
├── relay/worker/            # Cloudflare Worker WASM Watch Together relay
├── i18n/                    # Fluent (.ftl) catalog for all 13 languages
├── assets/                  # Canonical brand logos & icon generators
├── site/                    # anibeam.app website, wiki, and installer scripts
├── docs/                    # Living guides, ADRs, safety audit, and README media
├── Casks/                   # Homebrew cask (pinned at root for `brew tap`)
├── build.sh                 # One-command desktop build script
└── Cargo.toml               # Root Rust workspace manifest
```

## Crate layers

The Rust workspace is split into focused crates arranged in strict dependency layers so you can build and test one area (`cargo test -p anibeam-library`) without rebuilding the whole app:

```text
  Apps & Services     apps/linux (anibeam) · apps/phone · apps/cli · apps/relay · relay/worker
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
| `crates/host-api/` | `anibeam-host-api` | Wire records and Ed25519 request-proof auth for Tailscale host ↔ phone communication. |
| `crates/room/` | `anibeam-room` | Pure, I/O-free Watch Together room protocol, state machine, rate limits, and join HTML page. Shared by desktop, phone, native relay, and Cloudflare Worker. |
| `crates/recs/` | `anibeam-recs` | Pure recommendation scoring and ranking algorithm. |
| `crates/foundation/` | `anibeam-foundation` | Shared infrastructure: SQLite store & migrations (`src/store/sql/`), event bus, job tracker, paths, prefs, profiles, OS keyring / file secrets, secret redaction, and `Services`. |
| `crates/net/` | `anibeam-net` | Rate-limited HTTP clients (`src/limiter.rs`) for AniList, MAL, Jikan, TVmaze, AniSkip, ani.zip, IMDb, and Wikidata, plus DoH, domain fronting, and image caching. |
| `crates/library/` | `anibeam-library` | Sources, folder walk, filesystem watcher, episode filename classifier, quick fixes, history, delete/trash, and library read models (cards, details, snapshot). |
| `crates/releases/` | `anibeam-releases` | Release title parsing, RSS/Torznab/JSON feed readers, best-fit ranking, refinement grid, rules, tracker scrape stats, and signed endpoint repos. |
| `crates/metadata/` | `anibeam-metadata` | Auto-matching, metadata fetching, franchise crawl & closure (`src/franchise/`), Airing now, calendar feed, and Recommendations wiring (`src/recommended/`). |
| `crates/trackers/` | `anibeam-trackers` | AniList & MyAnimeList OAuth, list cache, two-way writes, local lists, Watching, move-up, On hold, and Planning lists. |
| `crates/downloads/` | `anibeam-downloads` | Built-in native downloads engine, `anirss` sidecar, qBittorrent client, rolling downloads, episode watch, pruning, and phone torrent engine (`src/phone/` behind `phone` feature). |
| `crates/playback/` | `anibeam-playback` | Session rules (30s view, 85% mark, completion), track memory, skip windows, mpv options, Discord Rich Presence (`src/presence/`), Chromecast/DLNA casting (`src/cast/`, `cast` feature), and Watch Together room client (`src/room/`). |
| `crates/host/` | `anibeam-host` | Tailscale library host server, peer trust, ffmpeg transcoding, streaming, and JSON library export/import (`src/transfer/`). |
| `crates/core/` | `anibeam-core` | Top-level facade assembling all crates into `Core` and its call router, plus launch, auto-updater, changelog parser, schedule agent, watching stats, UniFFI exports, and end-to-end integration tests (`crates/core/tests/`). |

## Standalone Cargo workspaces

Two directories intentionally define their own `[workspace]` and `Cargo.lock` so `cargo test --workspace` at the repo root stays fast and does not require mobile NDKs or WASM toolchains:

- **`apps/phone/`** (members: `.` and `companion/`): builds the Android native library and iOS executable on top of `anibeam-core` with `--features phone,cast`.
- **`relay/worker/`**: builds the `wasm32-unknown-unknown` Cloudflare Worker on top of `anibeam-room`.

## Where do I go to change…

| Task | Start here |
| --- | --- |
| A desktop screen, control, or theme | `apps/linux/qml/`, `apps/linux/themes/`, and `apps/linux/src/bridge/` |
| Desktop video player or C++/Obj-C++ glue | `apps/linux/cpp/` (`videoitem.*`, `nowplaying.mm`) and `apps/linux/mpv.conf` |
| Desktop installers / OS packaging | `apps/linux/packaging/{linux,macos,windows,aur,brew}/` and `Casks/anibeam.rb` |
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
| Watch Together relay servers | `apps/relay/` (self-hosted) and `relay/worker/` (Cloudflare Worker) |
| Website, wiki, or What's new pages | `site/` (`apps/linux/CHANGELOG.md` and `apps/phone/CHANGELOG.md` feed What's new) |
| Security & trust surface audit | [docs/safety-audit.md](docs/safety-audit.md) |

## Cross-cutting invariants

Every contributor (human or AI) should preserve these rules:

- **Rate limiting:** Every outbound provider call (AniList, MAL, Jikan, TVmaze, AniSkip, etc.) goes through the shared limiter in `crates/net/src/limiter.rs`. Never bypass it.
- **Signal-only activity log:** Record state changes, never per-asset, per-candidate, or per-browse chatter.
- **Franchise closure boundaries:** `crates/metadata/src/franchise/closure.rs` draws `CHARACTER` and `OTHER` edges but never walks across them, preventing crossover cameos from merging unrelated franchises.
- **Keep provider score scales on write:** `average_score` is stored as received from the provider and normalized at read time.
- **No polling the library:** Use the filesystem watcher plus a catch-up scan at launch—no periodic rescan intervals.
- **One window, play on when hidden:** Never open a second window or external mpv process, and never auto-pause when the window is hidden.
- **Self-contained desktop behavior:** No compositor-specific coupling (e.g., no `hyprctl` or Hyprland window rules).
- **Native desktop scrolling:** Every `Flickable` uses `WheelStep.qml`'s wheel handling (100 px per notch glided to one target).
- **No shipped download sources:** Nothing in this repo names or points at a download site—not in code, defaults, tests, fixtures, or docs. The core defines generic endpoint *kinds* (RSS, Torznab, JSON) only; users add signed endpoint repos themselves. Enforced by `.github/no-site-names.sh`.
- **All UI copy lives in `i18n/`:** Shells never hardcode user-facing English strings; code looks up Fluent message IDs. Enforced by `.github/strings-check.py`.
- **Curated patch notes:** Regular PRs do not edit `CHANGELOG.md`. Curate `apps/linux/CHANGELOG.md` and `apps/phone/CHANGELOG.md` before releases; `site/whats-new.py` builds `anibeam.app/whats-new/` directly from them.
