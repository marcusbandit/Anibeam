# Rust Core Crates

Shared Rust libraries that implement all Anibeam business logic behind `anibeam-core`. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the dependency graph and cross-cutting invariants.

## Layers (bottom → top)

- **Leaves (zero workspace deps):** `contract` (public API & `i18n/` loader), `host-api` (Tailscale wire protocol & Ed25519 proofs), `room` (Watch Together protocol & state machine), `recs` (recommendation ranking).
- **Platform:** `foundation` (SQLite store/migrations, bus, jobs, prefs, profiles, secrets), `net` (rate-limited HTTP clients, DoH, fronting, image cache).
- **Domains:** `library` → `releases` / `metadata` → `trackers` → `downloads` → `playback` / `host`.
- **Facade:** `core` (`anibeam-core`: `Core`, call router, launch, updater, changelog, schedule agent, stats, UniFFI exports, and end-to-end integration tests in `core/tests/`).

Iterate on a single crate directly instead of the whole workspace:

```bash
cargo test -p anibeam-library
cargo clippy -p anibeam-library --all-targets -- -D warnings
```
