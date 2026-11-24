# Applications

User-facing Anibeam shells built on top of `crates/core` (`anibeam-core`):

- [`desktop/`](desktop/) (`anibeam`) — Qt 6 + QML + libmpv desktop app for Linux, macOS, and Windows, including OS packaging scripts under `desktop/packaging/`.
- [`phone/`](phone/) (`anibeam-android` / `anibeam-ios`) — Slint + libmpv mobile app for Android and iOS, plus the `phone/companion/` Tailscale client (standalone Cargo workspace).
- [`cli/`](cli/) (`anibeam-cli`) — Terminal interface over `anibeam-core` for scripting, debugging, and headless library operations.

See [ARCHITECTURE.md](../ARCHITECTURE.md) for the full repository map.
