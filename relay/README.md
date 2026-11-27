# Relays and the dev Worker

Both relay implementations drive the same I/O-free room state machine in [`crates/room`](../crates/room/) and expose identical HTTP/WebSocket routes:

- [`server/`](server/) (`anibeam-relay`): self-hosted native Rust relay binary (part of the root Cargo workspace; also used in-process by `crates/core` integration tests).
- [`worker/`](worker/) (`anibeam-rooms-worker`): Cloudflare Worker WASM relay powering `rooms.anibeam.app` (standalone Cargo workspace targeting `wasm32-unknown-unknown`).

One more Worker sits here because it deploys the same way, though it has nothing to do with rooms:

- [`dev/`](dev/) (`anibeam-dev-worker`): the developer endpoints at `dev.anibeam.app`, endpoints only and no pages. `POST /usage` takes the usage log sends; every other route needs the owner's token. Standalone Cargo workspace targeting `wasm32-unknown-unknown`. Reading it: [docs/agents/dev-endpoint.md](../docs/agents/dev-endpoint.md).
