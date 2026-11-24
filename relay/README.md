# Watch Together relays

Both relay implementations drive the same I/O-free room state machine in [`crates/room`](../crates/room/) and expose identical HTTP/WebSocket routes:

- [`server/`](server/) (`anibeam-relay`): self-hosted native Rust relay binary (part of the root Cargo workspace; also used in-process by `crates/core` integration tests).
- [`worker/`](worker/) (`anibeam-rooms-worker`): Cloudflare Worker WASM relay powering `rooms.anibeam.app` (standalone Cargo workspace targeting `wasm32-unknown-unknown`).
