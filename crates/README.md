# Rust core crates

Shared Rust libraries that implement all Anibeam business logic behind `anibeam-core`. See [ARCHITECTURE.md](../ARCHITECTURE.md) for the dependency graph and cross-cutting invariants.

## Layers (bottom to top)

- Leaves with zero workspace dependencies: `contract` (public API and `i18n/` loader), `host-api` (Tailscale wire protocol and Ed25519 proofs), `room` (Watch Together protocol and state machine), and `recs` (recommendation ranking).
- Platform infrastructure: `foundation` (SQLite store and migrations, bus, jobs, prefs, profiles, secrets) and `net` (rate-limited HTTP clients, DoH, fronting, image cache).
- Domain crates: `library`, then `releases` and `metadata`, then `trackers`, then `downloads`, then `playback` and `host`.
- Core facade: `core` (`anibeam-core`: `Core`, call router, launch, updater, changelog, schedule agent, stats, UniFFI exports, and end-to-end integration tests in `core/tests/`).

Iterate on a single crate directly instead of the whole workspace:

```bash
cargo test -p anibeam-library
cargo clippy -p anibeam-library --all-targets -- -D warnings
```
