# Safety audit guide

This guide is a map for people and AI agents checking whether Anibeam is safe to install and run. It covers only the security and trust surface (installers, updates, network traffic, credentials, subprocesses, local listeners, filesystem access, external inputs, mobile permissions, and release provenance), not the rest of the application.

**Treat this file as an index, not a guarantee.** Read the referenced files directly, and run the whole-repo verification commands at the end to confirm nothing sits outside the paths listed here.

## At a glance

- **License and scope:** GPL-3.0 local anime library, player, and tracker for Linux, macOS, Windows, Android, and iOS.
- **No accounts or telemetry:** Anibeam has no user accounts, analytics, ads, telemetry, or crash-reporting server. See `site/privacy/index.html` for the user-facing summary.
- **No bundled download sites:** Nothing in this repository names or ships a download source (enforced on every PR by `.github/no-site-names.sh`). Download sources are added only by the user.
- **Unsigned desktop / sideloaded mobile releases:** Windows and macOS releases are currently unsigned/unnotarized (triggering SmartScreen and Gatekeeper), and mobile releases ship as a sideloaded Android APK and unsigned iOS IPA. All releases are built in public GitHub Actions workflows from tagged commits, and every third-party native binary fetched during packaging is pinned by SHA-256 or git commit hash.
- **Behaviors that often trigger automated flags, explained up front:**
  1. `core/src/update/install.rs` uses a short `/bin/sh` or hidden `powershell.exe` helper on exit during user-triggered updates (after verifying the file size and GitHub SHA-256 digest).
  2. `downloads/src/managed.rs` starts a bundled `qbittorrent-nox` bound to `127.0.0.1` only if built-in downloads are used, and leaves it running after Anibeam exits so RSS subscriptions continue.
  3. `foundation/src/process.rs` has a `shell()` helper (`sh -c` / `cmd /S /C`), called in one place (`downloads/src/jackett.rs`) only after user confirmation to run a user-configured local proxy command.
  4. `core/src/agent.rs` and `library/src/report.rs` can invoke a locally installed `claude` or `codex` CLI (never bundled) in a sandboxed empty temp folder when the user explicitly requests an episode schedule lookup or quick-fix bug report triage.
  5. `net/src/doh.rs` and `net/src/front.rs` implement DoH (phone builds, skipping local/tailnet names) and optional domain fronting (only for user-installed endpoint definitions that specify a `front` host, never for core requests).

## 1. Installers, packages, and release provenance

| Area | Where to look | What to verify |
| --- | --- | --- |
| Linux / macOS quick installer | `site/install.sh` | Entire script runs via `main "$@"` at the bottom (truncated downloads execute nothing); `sudo` is requested only for `apt`/`dnf`; macOS clears `com.apple.quarantine` on the unnotarized `.app`. |
| Windows installer | `apps/linux/packaging/windows/anibeam.iss` | Inno Setup with `PrivilegesRequired=lowest` (per-user install, no UAC prompt); `[UninstallRun]` runs PowerShell only to stop a bundled `qbittorrent-nox` whose path is inside `{app}\*`. |
| Homebrew cask & AUR | `Casks/anibeam.rb`, `apps/linux/packaging/aur/anibeam/PKGBUILD`, `apps/linux/packaging/aur/anibeam-bin/PKGBUILD` | Exact GitHub release download URLs and SHA-256 hashes (or source build). |
| Linux apt & dnf repos | `apps/linux/packaging/linux/publish-repos.sh`, `apps/linux/packaging/linux/keys/anibeam-packages.asc` | Repo metadata is GPG-signed from stable GitHub release `.deb`/`.rpm` assets. |
| Public CI build pipelines | `.github/workflows/release.yml`, `.github/workflows/nightly.yml`, `.github/workflows/linux.yml`, `.github/workflows/macos.yml`, `.github/workflows/windows.yml`, `.github/workflows/arch.yml`, `.github/workflows/android-apk.yml`, `.github/workflows/ios-ipa.yml` | Every shipped artifact is built from a git commit on GitHub-hosted runners and published as a draft until all assets are uploaded. |
| Third-party native binaries | `apps/linux/packaging/download.sh`, `apps/linux/packaging/linux/deps.sh`, `apps/linux/packaging/linux/appimage.sh`, `apps/linux/packaging/linux/packages.sh`, `apps/linux/packaging/macos/deps.sh`, `apps/linux/packaging/macos/anirss.sh`, `apps/linux/packaging/windows/deps.sh`, `apps/linux/packaging/windows/anirss.sh`, `apps/phone/scripts/fetch-mpv.sh`, `apps/phone/scripts/fetch-mpv-ios.sh` | Every downloaded dependency (libmpv, FFmpeg, MpvQt, qBittorrent-nox, python-build-standalone, `anirss`, `linuxdeploy`, `nfpm`, mpv-android, MPVKit) is verified against a pinned SHA-256 hash or git commit SHA. |
| Building from source | `build.sh`, `docs/build-from-source.md` | How to compile locally instead of installing a prebuilt binary. |

## 2. Auto-updates

| File | What to verify |
| --- | --- |
| `core/src/update.rs` | Update checks query only `api.github.com/repos/marcusbandit/Anibeam/releases` (AppImage/macOS/Windows/phone), `aur.archlinux.org/rpc/v5/info` (AUR installs), or `anibeam.app/repos.json` (apt/dnf installs). Checks only notify; nothing downloads automatically. |
| `core/src/update/install.rs` | Runs only when the user clicks to install an update. Downloads to a `.part` file, verifies exact byte size and GitHub's `sha256:` asset digest (`download()`), and stages the swap. On exit, spawns a minimal `/bin/sh` or `powershell.exe` helper to wait for the old PID and replace the install. System package installs (`/usr`, `/opt`) and source builds are never self-updated. |

## 3. Outbound network destinations and privacy

Every hardcoded external host in non-test source code belongs to one of these categories:

| Purpose | Host(s) | Where to look |
| --- | --- | --- |
| Metadata, posters, ratings, skip times | `graphql.anilist.co`, `api.tvmaze.com`, `api.jikan.moe`, `api.aniskip.com`, `datasets.imdbws.com`, `query.wikidata.org`, `thetvdb.com` (series and artwork pages, for clear logo languages), `artworks.thetvdb.com` (the logo files) | `net/src/anilist.rs`, `net/src/tvmaze.rs`, `net/src/jikan.rs`, `net/src/aniskip.rs`, `net/src/imdb.rs`, `net/src/wikidata.rs`, `net/src/tvdb.rs`, `net/src/images.rs` |
| Tracker OAuth & list sync (opt-in) | `anilist.co`, `myanimelist.net`, `api.myanimelist.net` | `trackers/src/oauth.rs`, `trackers/src/accounts.rs`, `trackers/src/cache.rs`, `trackers/src/writes.rs` |
| Update checks | `api.github.com`, `aur.archlinux.org`, `anibeam.app` | `core/src/update.rs` |
| Watch together relay (only when creating/joining a room) | `rooms.anibeam.app` (default `ROOM_RELAY`, configurable to self-host) | Default in `contract/src/records.rs`; relay server code in `relay/worker/src/lib.rs` (deletes room Durable Object storage on close) and `apps/relay/src/`; client crypto in `playback/src/room/seal.rs` (X25519 + HKDF-SHA256 + XChaCha20-Poly1305 for peer payloads; note comment on unauthenticated relay-relayed member keys). |
| Discord Rich Presence (off by default) | Local Discord IPC socket; `cdn.discordapp.com` for avatars | `playback/src/presence/discord.rs` |
| Phone DoH & endpoint domain fronting | `1.1.1.1/dns-query`, `8.8.8.8/resolve` (phone builds only); user-defined `front` hosts only | `net/src/doh.rs` (`is_home_name` keeps local/LAN/`.ts.net` lookups off public resolvers), `net/src/front.rs` (applies only to user-installed endpoint requests, disables redirects). |
| Pre-filled bug reports (browser only, user submits) | `github.com/marcusbandit/Anibeam/issues/new` | `library/src/report.rs` |
| No built-in download sites | None | `.github/no-site-names.sh`, `.github/workflows/no-sites.yml` |

## 4. Credentials, secrets, and log redaction

| File | What to verify |
| --- | --- |
| `foundation/src/secrets.rs` | Uses the OS keyring (Linux Secret Service over D-Bus, macOS Keychain, Windows Credential Manager) for tracker tokens, endpoint credentials, and qBittorrent passwords. Falls back to an atomically written `0600` `secrets.json` only when no keyring is available, deletes the losing store copy on every write, and never stores secrets in SQLite. |
| `foundation/src/redact.rs` | `scrub_secrets()` strips `user:password@` URL logins and known secret query/fragment parameters (`apikey`, `token`, `passkey`, `password`, etc., plain or percent-encoded) before events are stored. |
| `trackers/src/oauth.rs` | Binds a temporary `127.0.0.1` callback listener (5-minute timeout, 8 KB request cap). Emits MAL authorize URLs (which include the PKCE verifier) at `Debug` level so they are never persisted to the SQLite events table. |
| `host/src/keys.rs`, `foundation/src/paths.rs` | Stores the per-device Ed25519 key for Tailscale/device pairing in an owner-only `device-key` file inside the app data directory. |

## 5. Subprocesses, background processes, and external CLIs

| File | What it spawns and when |
| --- | --- |
| `foundation/src/process.rs` | Shared spawn helpers (`command` and `shell`). |
| `downloads/src/managed.rs` | Bundled `qbittorrent-nox`, started only when built-in downloads are used. Configured with `WebUI\Address=127.0.0.1` and `WebUI\LocalHostAuth=false` (loopback only), profile in `<data>/qbittorrent`. Intentionally detached so RSS subscriptions continue after closing Anibeam. |
| `downloads/src/anirss.rs` | Local `anirss` helper process (loopback only). |
| `downloads/src/jackett.rs` | The only non-test caller of `process::shell` (`sh -c` / `cmd /S /C`). Runs only after user confirmation, using a command from the user's own `anirss` config or `jackett_command` setting (defaulting to `jackett`). Endpoint repos cannot supply shell commands. |
| `core/src/agent.rs`, `library/src/report.rs` | Optional features that invoke a locally installed `claude` or `codex` binary (never bundled, only on explicit user action). Both run inside a fresh empty temp directory with `--safe-mode`, `--no-session-persistence`, restricted tools (`WebSearch,WebFetch` for schedules; `--tools ""` for Claude or `--sandbox read-only -c features.shell_tool=false` for Codex triage), timeouts, and process-group teardown. Results are shown for review and never auto-saved or auto-submitted. |
| `host/src/lib.rs`, `host/src/convert.rs` | Calls `tailscale status --json` (when Tailscale sharing is on) and `ffmpeg` (when transcoding streams to a phone). |
| `apps/linux/packaging/linux/anibeam-host.service`, `apps/linux/src/serve.rs` | Optional Linux headless host service (`anibeam serve`). Never auto-enabled on install; requires manual `systemctl --user enable --now anibeam-host`. |
| `apps/linux/src/scheme.rs`, `apps/linux/src/discord_link.rs`, `apps/linux/src/old_name.rs` | On Linux (skipped under `--root`), updates only the user's own `.desktop` files in `$XDG_DATA_HOME/applications` via `update-desktop-database` / `xdg-mime` to register `anibeam://`, `magnet:`, and `discord-<id>://` handlers (and migrate older launcher names). |
| `apps/linux/src/control.rs`, `apps/linux/src/dbus/testbus.rs` | `anibeam ctl open` starts the Anibeam binary itself if no window is running; `testbus.rs` spawns `dbus-daemon` in tests only. |

## 6. Local servers and IPC sockets

| Listener | Binding & scope | Where to look |
| --- | --- | --- |
| Tailscale library host (off by default) | Tailscale interface IPs only (never LAN/public unless overridden by `ANIBEAM_HOST_BIND` for dev/tests). Enforces `Host` header check (`known_host` against DNS rebinding), `trusted` middleware (`tailscale status --json` + per-device disable switches + Ed25519 per-request signatures with 300 s window and nonce replay cache), and integer ID lookups instead of raw file paths. | `host/src/lib.rs`, `host/src/server.rs`, `host/src/trust.rs`, `host/src/keys.rs`, `host-api/src/proof.rs` |
| Casting HTTP server & SSDP discovery (only while casting) | Binds on LAN only during an active cast to Chromecast/DLNA. Serves only the active media item behind an unguessable `/c/{token}` URL and restricts playlist segments to relative names (`read_pieces`); `discover.rs` uses an ephemeral UDP socket for SSDP discovery. | `playback/src/cast/server.rs`, `playback/src/cast/discover.rs`, `playback/src/cast/mod.rs` |
| BitTorrent tracker scrape UDP socket | Ephemeral `0.0.0.0:0` UDP socket used only to query seed/leech counts for release search results. | `releases/src/stats.rs` |
| Standalone watch-together relay | Separate self-hostable server binary (`anibeam-relay`); not started by the desktop or phone app. | `apps/relay/src/main.rs`, `apps/relay/src/lib.rs` |
| OAuth callback server (only during sign-in) | `127.0.0.1` temporary port, 5-minute timeout. | `trackers/src/oauth.rs` |
| Desktop control socket (Unix only) | `anibeam-control.sock` in the runtime directory with `0600` permissions (owner-only, not network-exposed). Accepts `anibeam ctl` commands including QML JS `eval` from the same local user account. | `apps/linux/src/control.rs` |
| Service handoff socket (Unix only) | Coordinates the single-instance lock between the GUI window and `anibeam serve`. | `apps/linux/src/serve.rs` |

## 7. Filesystem scope and destructive actions

| File | What to verify |
| --- | --- |
| `foundation/src/paths.rs` | All state stays under standard XDG / macOS Application Support / Windows AppData directories, or a single isolated directory with `--root <dir>`. Outside those directories, Anibeam only reads user-configured library folders (and optionally the user's `mpv.conf` when "Use my mpv.conf" is enabled). |
| `library/src/delete.rs`, `library/src/trash.rs` | Deleting episodes verifies every target path (including subtitle sidecars) is absolute, has no `..` (`ParentDir`) components, belongs to the requested series, and sits strictly inside a configured library source folder before moving to OS trash on desktop (or deleting on phone, which has no trash). |
| `downloads/src/prune.rs` | Optional auto-pruning of finished shows removes only the RSS subscription rule; it never touches library files or torrents. |

## 8. Untrusted external inputs (endpoint repos)

| File | What to verify |
| --- | --- |
| `releases/src/repos/index.rs`, `releases/src/repos/mod.rs` | Endpoint repos are signed JSON envelopes (`anibeam-endpoint-repo` v1, 1 MiB limit) verified with strict Ed25519 (`verify_strict`), key-pinned on first trust, and rollback-protected by monotonic `serial`. Definitions are declarative data only (no scripts or executables), require HTTPS (except `{host}` for local Torznab or user-typed custom URLs), reject placeholder hostnames so credentials cannot be redirected, and re-prompt before reusing credentials if an update changes host, `front`, or downgrades from HTTPS (`keeps_credentials`). |

## 9. Mobile permissions and code (Android & iOS)

| File | What to verify |
| --- | --- |
| `apps/phone/android/AndroidManifest.xml` | Requests only `INTERNET`, `ACCESS_NETWORK_STATE`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `POST_NOTIFICATIONS`, `ACCESS_WIFI_STATE`, and `CHANGE_WIFI_MULTICAST_STATE`. No contacts, location, camera, microphone, or broad storage permissions. Sets `android:allowBackup="false"`. Note `android:usesCleartextTraffic="true"` is enabled for HTTP streaming over Tailscale (WireGuard-encrypted at the network layer) and local LAN casting/indexers. |
| `apps/phone/android/java/com/marcusrosado/anibeam/` | Eight small Java helper classes for screen/bars/fold crease, icon aliases, Android's SAF file picker (`PickActivity.java`), download foreground notification (`TorrentService.java`), and `adb` test control (`Control.java`). |
| `apps/phone/src/control/mod.rs`, `apps/phone/android/java/com/marcusrosado/anibeam/Control.java`, `apps/phone/build.rs` | The `com.marcusrosado.anibeam.CONTROL` broadcast receiver requires `android.permission.DUMP` (held only by `adb` shell and system) and is compiled out of Play Store builds (`cfg(control)`). |
| `apps/phone/ios/Info.plist` | Declares only `NSLocalNetworkUsageDescription`, `_googlecast._tcp` Bonjour discovery, and `audio` background mode. |

## 10. Build scripts, `unsafe` Rust, and tracked binaries

- **Cargo build scripts:** `core/build.rs`, `net/build.rs`, `library/build.rs`, `trackers/build.rs`, `playback/build.rs`, `apps/linux/build.rs`, `apps/phone/build.rs`, and the shared helper `trackers/env_local.rs`. None access the network or download code; they set version/target cfgs, optionally read OAuth/Discord public client IDs from env or `.env.local`, and link Qt/MpvQt/Slint/libmpv. (`host/src/build.rs` is a regular snapshot-builder module, not a build script; `spikes/` are non-workspace prototypes.)
- **`unsafe` Rust:** Across all 14 core library crates, there is a single `unsafe` call (`PeekNamedPipe` for Windows Discord IPC in `playback/src/presence/discord.rs`). All remaining `unsafe` blocks are FFI boundaries in the desktop and phone shells (`cxx-qt` bridges in `apps/linux/src/bridge/` and `apps/linux/build.rs`, macOS Now Playing/IOKit in `apps/linux/src/macnp.rs` and `iokit.rs`, Windows power APIs in `apps/linux/src/winpower.rs`, pre-thread `std::env::set_var` at startup in `apps/linux/src/main.rs` and `paths.rs`, and Android/iOS JNI, Metal/OpenGL, and `libmpv` FFI in `apps/phone/src/{mpv.rs,thumbs.rs,video/,platform/,control/android.rs}`).
- **Compiled binaries in git:** Only two small Qt shader bytecode files (`apps/linux/shaders/poster.vert.qsb`, `poster.frag.qsb`), compiled from `poster.vert` and `poster.frag` beside them by `apps/linux/shaders/build.sh`.

## Independent whole-repo verification commands

Run these from the repository root to verify nothing relevant exists outside the files indexed above:

```bash
# 1. Every process spawn in Rust source (excluding integration tests)
git grep -n -E 'Command::new|process::shell' -- '*.rs' ':!*tests*' ':!spikes/*'

# 2. Every network listener or socket bind
git grep -n -E 'TcpListener|UdpSocket|UnixListener|axum::serve' -- '*.rs' ':!*tests*' ':!spikes/*'

# 3. Every hardcoded URL in non-test Rust source (inspect alongside net/src/ and core/src/update.rs)
git grep -n -E 'https?://|wss?://' -- '*.rs' ':!*tests*' ':!spikes/*'

# 4. Every Cargo build script and every unsafe block in Rust source
git ls-files '*build.rs'
git grep -n 'unsafe ' -- '*.rs' ':!*tests*' ':!spikes/*'

# 5. Any compiled or archive binaries tracked in git
git ls-files | grep -iE '\.(so|dll|dylib|exe|apk|ipa|jar|bin|o|a|qsb|wasm|pyc|zip|tar|gz|7z|deb|rpm|dmg|AppImage)$'

# 6. Confirm zero download sites are named anywhere in the repo
.github/no-site-names.sh
```

If you find a real security issue during an audit, please report it privately as described in [SECURITY.md](../SECURITY.md).
