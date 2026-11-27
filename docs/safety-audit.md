# Safety audit guide

This guide is a map for people and AI agents checking whether Anibeam is safe to install and run. It covers only the security and trust surface (installers, updates, network traffic, credentials, subprocesses, local listeners, filesystem access, external inputs, mobile permissions, and release provenance), not the rest of the application.

**Treat this file as an index, not a guarantee.** Read the referenced files directly, and run the whole-repo verification commands at the end to confirm nothing sits outside the paths listed here.

## At a glance

- **License and scope:** GPL-3.0 local anime library, player, and tracker for Linux, macOS, Windows, Android, and iOS.
- **No accounts or telemetry:** Anibeam has no user accounts, analytics, ads, telemetry, or crash-reporting server. It keeps a local usage log (counts of calls, pages and errors per day in `usage.json`, nothing personal) that never leaves the machine unless its owner presses send, once, in Settings > About or through `anibeam-cli usage send --yes`. See `site/privacy/index.html` for the user-facing summary.
- **No bundled download sites:** Nothing in this repository names or ships a download source (enforced on every PR by `scripts/ci/no-site-names.sh`). Download sources are added only by the user.
- **Unsigned desktop / sideloaded mobile releases:** Windows and macOS releases are currently unsigned/unnotarized (triggering SmartScreen and Gatekeeper), and mobile releases ship as a sideloaded Android APK and unsigned iOS IPA. All releases are built in public GitHub Actions workflows from tagged commits, and every third-party native binary fetched during packaging is pinned by SHA-256 or git commit hash.
- **Behaviors that often trigger automated flags, explained up front:**
  1. `crates/core/src/update/install.rs` uses a short `/bin/sh` or hidden `powershell.exe` helper on exit during user-triggered updates (after verifying the file size and GitHub SHA-256 digest).
  2. `crates/downloads/src/managed.rs` starts a bundled `qbittorrent-nox` bound to `127.0.0.1` only if built-in downloads are used, and leaves it running after Anibeam exits so RSS subscriptions continue.
  3. `crates/foundation/src/process.rs` has a `shell()` helper (`sh -c` / `cmd /S /C`), called in one place (`crates/downloads/src/jackett.rs`) only after user confirmation to run a user-configured local proxy command.
  4. `crates/core/src/agent.rs` and `crates/library/src/report.rs` can invoke a locally installed `claude` or `codex` CLI (never bundled) in a sandboxed empty temp folder when the user explicitly requests an episode schedule lookup or quick-fix bug report triage.
  5. `crates/net/src/doh.rs` and `crates/net/src/front.rs` implement DoH (phone builds, skipping local/tailnet names) and optional domain fronting (only for user-installed endpoint definitions that specify a `front` host, never for core requests).

## 1. Installers, packages, and release provenance

| Area | Where to look | What to verify |
| --- | --- | --- |
| Linux / macOS quick installer | `site/install.sh` | Entire script runs via `main "$@"` at the bottom (truncated downloads execute nothing); `sudo` is requested only for `apt`/`dnf`; macOS clears `com.apple.quarantine` on the unnotarized `.app`. |
| Windows installer | `apps/desktop/packaging/windows/anibeam.iss` | Inno Setup with `PrivilegesRequired=lowest` (per-user install, no UAC prompt); `[UninstallRun]` runs PowerShell only to stop a bundled `qbittorrent-nox` whose path is inside `{app}\*`. |
| Homebrew cask & AUR | `Casks/anibeam.rb`, `apps/desktop/packaging/aur/anibeam/PKGBUILD`, `apps/desktop/packaging/aur/anibeam-bin/PKGBUILD` | Exact GitHub release download URLs and SHA-256 hashes (or source build). |
| Linux apt & dnf repos | `apps/desktop/packaging/linux/publish-repos.sh`, `apps/desktop/packaging/linux/keys/anibeam-packages.asc` | Repo metadata is GPG-signed from stable GitHub release `.deb`/`.rpm` assets. |
| Public CI build pipelines | `.github/workflows/release.yml`, `.github/workflows/nightly.yml`, `.github/workflows/linux.yml`, `.github/workflows/macos.yml`, `.github/workflows/windows.yml`, `.github/workflows/arch.yml`, `.github/workflows/android-apk.yml`, `.github/workflows/ios-ipa.yml` | Every shipped artifact is built from a git commit on GitHub-hosted runners and published as a draft until all assets are uploaded. |
| Third-party native binaries | `apps/desktop/packaging/download.sh`, `apps/desktop/packaging/linux/deps.sh`, `apps/desktop/packaging/linux/appimage.sh`, `apps/desktop/packaging/linux/packages.sh`, `apps/desktop/packaging/macos/deps.sh`, `apps/desktop/packaging/macos/anirss.sh`, `apps/desktop/packaging/windows/deps.sh`, `apps/desktop/packaging/windows/anirss.sh`, `apps/phone/scripts/fetch-mpv.sh`, `apps/phone/scripts/fetch-mpv-ios.sh` | Every downloaded dependency (libmpv, FFmpeg, MpvQt, qBittorrent-nox, python-build-standalone, `anirss`, `linuxdeploy`, `nfpm`, mpv-android, MPVKit) is verified against a pinned SHA-256 hash or git commit SHA. |
| Building from source | `build.sh`, `docs/build-from-source.md` | How to compile locally instead of installing a prebuilt binary. |

## 2. Auto-updates

| File | What to verify |
| --- | --- |
| `crates/core/src/update.rs` | Update checks query only `api.github.com/repos/marcusbandit/Anibeam/releases` (AppImage/macOS/Windows/phone), `aur.archlinux.org/rpc/v5/info` (AUR installs), or `anibeam.app/repos.json` (apt/dnf installs). Checks only notify; nothing downloads automatically. |
| `crates/core/src/update/install.rs` | Runs only when the user clicks to install an update. Downloads to a `.part` file, verifies exact byte size and GitHub's `sha256:` asset digest (`download()`), and stages the swap. On exit, spawns a minimal `/bin/sh` or `powershell.exe` helper to wait for the old PID and replace the install. System package installs (`/usr`, `/opt`) and source builds are never self-updated. |

## 3. Outbound network destinations and privacy

Every hardcoded external host in non-test source code belongs to one of these categories:

| Purpose | Host(s) | Where to look |
| --- | --- | --- |
| Metadata, posters, ratings, skip times | `graphql.anilist.co`, `api.tvmaze.com`, `api.jikan.moe`, `api.aniskip.com`, `datasets.imdbws.com`, `query.wikidata.org`, `thetvdb.com` (series and artwork pages, for clear logo languages), `artworks.thetvdb.com` (the logo files) | `crates/net/src/anilist.rs`, `crates/net/src/tvmaze.rs`, `crates/net/src/jikan.rs`, `crates/net/src/aniskip.rs`, `crates/net/src/imdb.rs`, `crates/net/src/wikidata.rs`, `crates/net/src/tvdb.rs`, `crates/net/src/images.rs` |
| Tracker OAuth & list sync (opt-in) | `anilist.co`, `myanimelist.net`, `api.myanimelist.net` | `crates/trackers/src/oauth.rs`, `crates/trackers/src/accounts.rs`, `crates/trackers/src/cache.rs`, `crates/trackers/src/writes.rs` |
| Update checks | `api.github.com`, `aur.archlinux.org`, `anibeam.app` | `crates/core/src/update.rs` |
| Watch together relay (only when creating/joining a room) | `rooms.anibeam.app` (default `ROOM_RELAY`, configurable to self-host) | Default in `crates/contract/src/records.rs`; relay server code in `relay/worker/src/lib.rs` (deletes room Durable Object storage on close) and `relay/server/src/`; client crypto in `crates/playback/src/room/seal.rs` (X25519 + HKDF-SHA256 + XChaCha20-Poly1305 for peer payloads; note comment on unauthenticated relay-relayed member keys). |
| Usage log send (only when the owner presses send) | `dev.anibeam.app/usage` (always `DEV_ENDPOINT` in `crates/contract/src/records.rs`, never a host from the preferences) | `crates/core/src/usage.rs` is the only code that sends it, from the `SendUsage` call alone; the counts and the file are `crates/foundation/src/usage.rs` (no paths, titles, names, accounts, addresses or install id, and a fresh random send id each time that is never stored). The route is `relay/dev/src/usage.rs` on its own Worker, apart from the rooms relay (256 KiB cap, shape check, per address rate limit, 2000 sends kept); reading and pruning pass the one token check in `relay/dev/src/auth.rs` (the `DEV_READ_TOKEN` Worker secret), and unknown paths are 404. |
| Discord Rich Presence (off by default) | Local Discord IPC socket; `cdn.discordapp.com` for avatars | `crates/playback/src/presence/discord.rs` |
| Phone DoH & endpoint domain fronting | `1.1.1.1/dns-query`, `8.8.8.8/resolve` (phone builds only); user-defined `front` hosts only | `crates/net/src/doh.rs` (`is_home_name` keeps local/LAN/`.ts.net` lookups off public resolvers), `crates/net/src/front.rs` (applies only to user-installed endpoint requests, disables redirects). |
| Pre-filled bug reports (browser only, user submits) | `github.com/marcusbandit/Anibeam/issues/new` | `crates/library/src/report.rs` |
| No built-in download sites | None | `scripts/ci/no-site-names.sh`, `.github/workflows/no-sites.yml` |

## 4. Credentials, secrets, and log redaction

| File | What to verify |
| --- | --- |
| `crates/foundation/src/secrets.rs` | Uses the OS keyring (Linux Secret Service over D-Bus, macOS Keychain, Windows Credential Manager) for tracker tokens, endpoint credentials, and qBittorrent passwords. Falls back to an atomically written `0600` `secrets.json` only when no keyring is available, deletes the losing store copy on every write, and never stores secrets in SQLite. |
| `crates/foundation/src/redact.rs` | `scrub_secrets()` strips `user:password@` URL logins and known secret query/fragment parameters (`apikey`, `token`, `passkey`, `password`, etc., plain or percent-encoded) before events are stored. |
| `crates/trackers/src/oauth.rs` | Binds a temporary `127.0.0.1` callback listener (5-minute timeout, 8 KB request cap). Emits MAL authorize URLs (which include the PKCE verifier) at `Debug` level so they are never persisted to the SQLite events table. |
| `crates/host/src/keys.rs`, `crates/foundation/src/paths.rs` | Stores the per-device Ed25519 key for Tailscale/device pairing in an owner-only `device-key` file inside the app data directory. |

## 5. Subprocesses, background processes, and external CLIs

| File | What it spawns and when |
| --- | --- |
| `crates/foundation/src/process.rs` | Shared spawn helpers (`command` and `shell`). |
| `crates/downloads/src/managed.rs` | Bundled `qbittorrent-nox`, started only when built-in downloads are used. Configured with `WebUI\Address=127.0.0.1` and `WebUI\LocalHostAuth=false` (loopback only), profile in `<data>/qbittorrent`. Intentionally detached so RSS subscriptions continue after closing Anibeam. |
| `crates/downloads/src/anirss.rs` | Local `anirss` helper process (loopback only). |
| `crates/downloads/src/jackett.rs` | The only non-test caller of `process::shell` (`sh -c` / `cmd /S /C`). Runs only after user confirmation, using a command from the user's own `anirss` config or `jackett_command` setting (defaulting to `jackett`). Endpoint repos cannot supply shell commands. |
| `crates/core/src/agent.rs`, `crates/library/src/report.rs` | Optional features that invoke a locally installed `claude` or `codex` binary (never bundled, only on explicit user action). Both run inside a fresh empty temp directory with `--safe-mode`, `--no-session-persistence`, restricted tools (`WebSearch,WebFetch` for schedules; `--tools ""` for Claude or `--sandbox read-only -c features.shell_tool=false` for Codex triage), timeouts, and process-group teardown. Results are shown for review and never auto-saved or auto-submitted. |
| `crates/host/src/lib.rs`, `crates/host/src/convert.rs` | Calls `tailscale status --json` (when Tailscale sharing is on) and `ffmpeg` (when transcoding streams to a phone). |
| `apps/desktop/packaging/linux/anibeam-host.service`, `apps/desktop/src/serve.rs` | Optional Linux headless host service (`anibeam serve`). Never auto-enabled on install; requires manual `systemctl --user enable --now anibeam-host`. |
| `apps/desktop/src/scheme.rs`, `apps/desktop/src/discord_link.rs`, `apps/desktop/src/old_name.rs` | On Linux (skipped under `--root`), updates only the user's own `.desktop` files in `$XDG_DATA_HOME/applications` via `update-desktop-database` / `xdg-mime` to register `anibeam://`, `magnet:`, and `discord-<id>://` handlers (and migrate older launcher names). |
| `apps/desktop/src/control.rs`, `apps/desktop/src/dbus/testbus.rs` | `anibeam ctl open` starts the Anibeam binary itself if no window is running; `testbus.rs` spawns `dbus-daemon` in tests only. |

## 6. Local servers and IPC sockets

| Listener | Binding & scope | Where to look |
| --- | --- | --- |
| Tailscale library host (off by default) | Tailscale interface IPs only (never LAN/public unless overridden by `ANIBEAM_HOST_BIND` for dev/tests). Enforces `Host` header check (`known_host` against DNS rebinding), `trusted` middleware (`tailscale status --json` + per-device disable switches + Ed25519 per-request signatures with 300 s window and nonce replay cache), and integer ID lookups instead of raw file paths. | `crates/host/src/lib.rs`, `crates/host/src/server.rs`, `crates/host/src/trust.rs`, `crates/host/src/keys.rs`, `crates/host-api/src/proof.rs` |
| Casting HTTP server & SSDP discovery (only while casting) | Binds on LAN only during an active cast to Chromecast/DLNA. Serves only the active media item behind an unguessable `/c/{token}` URL and restricts playlist segments to relative names (`read_pieces`); `discover.rs` uses an ephemeral UDP socket for SSDP discovery. | `crates/playback/src/cast/server.rs`, `crates/playback/src/cast/discover.rs`, `crates/playback/src/cast/mod.rs` |
| BitTorrent tracker scrape UDP socket | Ephemeral `0.0.0.0:0` UDP socket used only to query seed/leech counts for release search results. | `crates/releases/src/stats.rs` |
| Standalone watch-together relay | Separate self-hostable server binary (`anibeam-relay`); not started by the desktop or phone app. | `relay/server/src/main.rs`, `relay/server/src/lib.rs` |
| OAuth callback server (only during sign-in) | `127.0.0.1` temporary port, 5-minute timeout. | `crates/trackers/src/oauth.rs` |
| Desktop control socket (Unix only) | `anibeam-control.sock` in the runtime directory with `0600` permissions (owner-only, not network-exposed). Accepts `anibeam ctl` commands including QML JS `eval` from the same local user account. | `apps/desktop/src/control.rs` |
| Service handoff socket (Unix only) | Coordinates the single-instance lock between the GUI window and `anibeam serve`. | `apps/desktop/src/serve.rs` |

## 7. Filesystem scope and destructive actions

| File | What to verify |
| --- | --- |
| `crates/foundation/src/paths.rs` | All state stays under standard XDG / macOS Application Support / Windows AppData directories, or a single isolated directory with `--root <dir>`. Outside those directories, Anibeam only reads user-configured library folders (and optionally the user's `mpv.conf` when "Use my mpv.conf" is enabled). |
| `crates/library/src/delete.rs`, `crates/library/src/trash.rs` | Deleting episodes verifies every target path (including subtitle sidecars) is absolute, has no `..` (`ParentDir`) components, belongs to the requested series, and sits strictly inside a configured library source folder before moving to OS trash on desktop (or deleting on phone, which has no trash). |
| `crates/downloads/src/prune.rs` | Optional auto-pruning of finished shows removes only the RSS subscription rule; it never touches library files or torrents. |

## 8. Untrusted external inputs (endpoint repos)

| File | What to verify |
| --- | --- |
| `crates/releases/src/repos/index.rs`, `crates/releases/src/repos/mod.rs` | Endpoint repos are signed JSON envelopes (`anibeam-endpoint-repo` v1, 1 MiB limit) verified with strict Ed25519 (`verify_strict`), key-pinned on first trust, and rollback-protected by monotonic `serial`. Definitions are declarative data only (no scripts or executables), require HTTPS (except `{host}` for local Torznab or user-typed custom URLs), reject placeholder hostnames so credentials cannot be redirected, and re-prompt before reusing credentials if an update changes host, `front`, or downgrades from HTTPS (`keeps_credentials`). |

## 9. Mobile permissions and code (Android & iOS)

| File | What to verify |
| --- | --- |
| `apps/phone/android/AndroidManifest.xml` | Requests only `INTERNET`, `ACCESS_NETWORK_STATE`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `POST_NOTIFICATIONS`, `ACCESS_WIFI_STATE`, and `CHANGE_WIFI_MULTICAST_STATE`. No contacts, location, camera, microphone, or broad storage permissions. Sets `android:allowBackup="false"`. Note `android:usesCleartextTraffic="true"` is enabled for HTTP streaming over Tailscale (WireGuard-encrypted at the network layer) and local LAN casting/indexers. |
| `apps/phone/android/java/com/marcusrosado/anibeam/` | Eight small Java helper classes for screen/bars/fold crease, icon aliases, Android's SAF file picker (`PickActivity.java`), download foreground notification (`TorrentService.java`), and `adb` test control (`Control.java`). |
| `apps/phone/src/control/mod.rs`, `apps/phone/android/java/com/marcusrosado/anibeam/Control.java`, `apps/phone/build.rs` | The `com.marcusrosado.anibeam.CONTROL` broadcast receiver requires `android.permission.DUMP` (held only by `adb` shell and system) and is compiled out of Play Store builds (`cfg(control)`). |
| `apps/phone/ios/Info.plist` | Declares only `NSLocalNetworkUsageDescription`, `_googlecast._tcp` Bonjour discovery, and `audio` background mode. |

## 10. Build scripts, `unsafe` Rust, and tracked binaries

- **Cargo build scripts:** `crates/core/build.rs`, `crates/net/build.rs`, `crates/library/build.rs`, `crates/trackers/build.rs`, `crates/playback/build.rs`, `apps/desktop/build.rs`, `apps/phone/build.rs`, and the shared helper `crates/trackers/env_local.rs`. None access the network or download code; they set version/target cfgs, optionally read OAuth/Discord public client IDs from env or `.env.local`, and link Qt/MpvQt/Slint/libmpv. (`crates/host/src/build.rs` is a regular snapshot-builder module, not a build script.)
- **`unsafe` Rust:** Across all 14 core library crates, there is a single `unsafe` call (`PeekNamedPipe` for Windows Discord IPC in `crates/playback/src/presence/discord.rs`). All remaining `unsafe` blocks are FFI boundaries in the desktop and phone shells (`cxx-qt` bridges in `apps/desktop/src/bridge/` and `apps/desktop/build.rs`, macOS Now Playing/IOKit in `apps/desktop/src/macnp.rs` and `iokit.rs`, Windows power APIs in `apps/desktop/src/winpower.rs`, pre-thread `std::env::set_var` at startup in `apps/desktop/src/main.rs` and `paths.rs`, and Android/iOS JNI, Metal/OpenGL, and `libmpv` FFI in `apps/phone/src/{mpv.rs,thumbs.rs,video/,platform/,control/android.rs}`).
- **Compiled binaries in git:** Only two small Qt shader bytecode files (`apps/desktop/shaders/poster.vert.qsb`, `poster.frag.qsb`), compiled from `poster.vert` and `poster.frag` beside them by `apps/desktop/shaders/build.sh`.

## Independent whole-repo verification commands

Run these from the repository root to verify nothing relevant exists outside the files indexed above:

```bash
# 1. Every process spawn in Rust source (excluding integration tests)
git grep -n -E 'Command::new|process::shell' -- '*.rs' ':!*tests*'

# 2. Every network listener or socket bind
git grep -n -E 'TcpListener|UdpSocket|UnixListener|axum::serve' -- '*.rs' ':!*tests*'

# 3. Every hardcoded URL in non-test Rust source (inspect alongside crates/net/src/ and crates/core/src/update.rs)
git grep -n -E 'https?://|wss?://' -- '*.rs' ':!*tests*'

# 4. Every Cargo build script and every unsafe block in Rust source
git ls-files '*build.rs'
git grep -n 'unsafe ' -- '*.rs' ':!*tests*'

# 5. Any compiled or archive binaries tracked in git
git ls-files | grep -iE '\.(so|dll|dylib|exe|apk|ipa|jar|bin|o|a|qsb|wasm|pyc|zip|tar|gz|7z|deb|rpm|dmg|AppImage)$'

# 6. Confirm zero download sites are named anywhere in the repo
scripts/ci/no-site-names.sh
```

If you find a real security issue during an audit, please report it privately as described in [SECURITY.md](../SECURITY.md).
