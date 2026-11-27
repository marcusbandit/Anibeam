# Dev endpoints on Cloudflare Workers

The project's developer endpoints at `dev.anibeam.app`, a Cloudflare Worker in
Rust built with [workers-rs](https://github.com/cloudflare/workers-rs). Endpoints
only, no pages. It is apart from the rooms relay in `relay/worker` on purpose:
nothing here has to do with rooms, and a room relay someone hosts themselves
never needs it.

| Route | Who | Answer |
|---|---|---|
| `POST /usage` `{"send_id", "usage"}` | anyone | `201` stored, `200` that send id is already stored, `400` not a usage log, `413` over 256 KiB, `429` over 5 a minute per address |
| `GET /usage` `?since=<unix ms>&after=<id>` | owner | `200 {"sends": [{"id", "at", "send_id", "usage"}], "next"}`, oldest first, 100 a page |
| `DELETE /usage?before=<unix ms>` | owner | `200 {"deleted": n}`, `400` without `before` |
| anything else | | `404` |

"Owner" means `Authorization: Bearer <DEV_READ_TOKEN>`. One check in
`src/auth.rs` guards every route except `POST /usage`: `401` with a wrong or
missing token, `503` while the Worker has no `DEV_READ_TOKEN` secret, so a
deploy that forgot it is closed, never open. A path the Worker does not know is
`404` before the token is looked at.

`since` reads only the sends received after that time; `at` is when the Worker
stored each one. `next` is the `after` for the following page, or null at the
end. The usage sends live in one SQLite-backed Durable Object, `UsageObject`,
one row per send keyed by its random `send_id`, the newest 2000 kept. Nothing
stores the sender's address: the limiter counts it and forgets it.

This crate is its own Cargo workspace, so `cargo test --workspace` at the repo
root never builds it.

## Reading it

Use `scripts/dev-endpoint` from the repo root, not curl by hand:

    scripts/dev-endpoint usage list
    scripts/dev-endpoint usage pull
    scripts/dev-endpoint usage summary

Every command, and how to add an endpoint, is in
[docs/agents/dev-endpoint.md](../../docs/agents/dev-endpoint.md).

## Build and test

    rustup target add wasm32-unknown-unknown
    cargo install worker-build
    cargo test
    ./smoke.sh

`cargo test` runs the unit tests on the host: the send check, the token check,
the public route and the query numbers. `smoke.sh` starts `wrangler dev` in
local mode with a token made for the run and drives it: the send rules, the
token gate, 404s, `since`, `DELETE`, the limit, and then `scripts/dev-endpoint`
itself (list, pull, a second pull, summary, a missing token, prune). Nothing
leaves the machine. Needs node 22 or later and python 3.

## Deploy

On the free plan, from this directory, logged in with the Cloudflare account
that owns `anibeam.app`:

1. `npx wrangler@4.161.0 deploy`. The first deploy creates the Worker, the
   `UsageObject` class from the `v1` migration, the `USAGE_LIMIT` limiter, and
   the custom domain `dev.anibeam.app` from `wrangler.toml`.
2. Give it a read token. The token is never in the repo or in
   `wrangler.toml`. From the repo root:

       scripts/dev-endpoint token new
       scripts/dev-endpoint token show | (cd relay/dev && npx wrangler@4.161.0 secret put DEV_READ_TOKEN)

   `token new` makes a random 64 character token and keeps it in the keyring
   (`secret-tool`, service `anibeam-dev`, account `read-token`).
3. Check it: `scripts/dev-endpoint usage list` answers with nothing (or the
   sends so far), not an error.

To change the token, run `token new --force` and step 2 again.
