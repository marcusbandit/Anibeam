# Dev endpoints (dev.anibeam.app)

`dev.anibeam.app` is the project's own Cloudflare Worker for developer endpoints, built from
`relay/dev/` (Worker name `anibeam-dev`). It has no pages. Today it has one endpoint, `/usage`,
where an app's usage log arrives when its owner presses send (#1532). More endpoints will join it
at `/<thing>`.

Only `POST /usage` is public. Every other route needs `Authorization: Bearer <token>`, where the
token is the Worker's `DEV_READ_TOKEN` secret. Unknown paths answer 404. The route table and
status codes are in [relay/dev/README.md](../../relay/dev/README.md).

## The rule for agents

- Read, pull and summarise the sends freely. They hold counts only: calls, pages and errors per
  day, no paths, titles, names or accounts.
- Never send a user's usage log (`anibeam-cli usage send --yes`, or a POST to `/usage` by any
  other means) without the owner's explicit yes in that conversation, after showing them the
  file. A yes from an earlier conversation does not count. See "Usage log" in
  [AGENTS.md](../../AGENTS.md).
- Never print the token into a chat, an issue, a PR or a log. `token show` is for piping only.
- Prune only when the owner asks for it. Pull first, so nothing is lost.
- Never commit pulled sends to the repo. They live under `~/.local/share/anibeam/usage-sends/`.

## The token

Read in this order:

1. `ANIBEAM_DEV_TOKEN` in the environment.
2. The keyring: `secret-tool lookup service anibeam-dev account read-token`.

When neither has it, every command that talks to the Worker stops with a message saying where to
put it. To store a token you were given, `dev-endpoint token set` and paste it, then Ctrl-D. To
make a fresh one and put it on the Worker, see Deploy in [relay/dev/README.md](../../relay/dev/README.md).

## scripts/dev-endpoint

Installed, it is `dev-endpoint` on `PATH` with tab completion (see "Installing" below). Otherwise run
`scripts/dev-endpoint` from the repo root. Every level takes `--help`.

    dev-endpoint usage list [--since DATE]
    dev-endpoint usage pull [DIR]
    dev-endpoint usage summary [DIR] [--top N]
    dev-endpoint usage prune --before DATE
    dev-endpoint token new [--force] | set | show

Global flags go before the group: `--endpoint URL` (or `ANIBEAM_DEV_ENDPOINT`) talks to another
Worker, such as a local `wrangler dev`; `--no-keyring` never reads the keyring. A DATE is
`2026-11-01`, `2026-11-01T12:00` (UTC) or unix milliseconds.

### usage list

One line per send on the Worker: when it arrived (UTC), app version, platform, days it covers,
total events, send id.

    $ dev-endpoint usage list
    2026-11-20 09:14:03Z  2.6.0                    desktop   12 days     348 events  0f3c9a1e5b7d42c8a6e1f09b3d5c7a21
    2026-11-26 18:40:51Z  2.6.0-nightly.20261126   desktop    3 days      97 events  7d21b6c4e8f0a3955b1c2d4e6f8a0b13

    $ dev-endpoint usage list --since 2026-11-25

### usage pull

Saves every send that arrived since the last pull, one JSON file each, named
`<received>-<send id>.json`, into `${XDG_DATA_HOME:-~/.local/share}/anibeam/usage-sends/` or the
folder given. The newest send's time goes in `.last-pull` in that folder, so the next pull fetches
only what is new. Run it before `summary`.

    $ dev-endpoint usage pull
    2 new sends saved to ~/.local/share/anibeam/usage-sends
    $ dev-endpoint usage pull
    no new sends since 2026-11-26 18:40:51Z

### usage summary

Sums every pulled send in the folder, largest first: versions, calls, pages and errors. It reads
local files only, so it works offline.

    $ dev-endpoint usage summary --top 3
    2 sends in ~/.local/share/anibeam/usage-sends

    versions (top 3 of 2)
      2.6.0 desktop                   1
      2.6.0-nightly.20261126 desktop  1

    calls (top 3 of 41)
      GetSeries       120
      ListSeries       88
      Scan             17

    pages (top 3 of 9)
      library         54
      show            40
      settings-about   3

    errors (top 3 of 2)
      Scan:NotFound    2
      SendUsage:Net    1

### usage prune

Deletes the sends that arrived before a date, on the Worker. Pulled files stay. The Worker also
keeps only the newest 2000 sends on its own.

    $ dev-endpoint usage prune --before 2026-11-01
    14 sends received before 2026-11-01 00:00:00Z deleted

## Installing

Link the script and its completions from the checkout you keep (the main one, not a worktree that
gets removed after a merge):

    ln -sfn "$PWD/scripts/dev-endpoint" ~/.local/bin/dev-endpoint
    ln -sfn "$PWD/scripts/completions/_dev-endpoint" ~/.local/share/zsh/site-functions/_dev-endpoint
    ln -sfn "$PWD/scripts/completions/dev-endpoint.bash" ~/.local/share/bash-completion/completions/dev-endpoint

`zsh -i -c 'print -r -- ${(ok)_comps[(I)dev-endpoint]}'` prints `dev-endpoint` once zsh sees it.

## Adding an endpoint

In `relay/dev/`:

1. Write `src/<thing>.rs` with `pub async fn route(req: Request, env: &Env) -> Result<Response>`
   that matches on the method and answers 405 for the rest. Storage is a SQLite-backed Durable
   Object, as `UsageObject` in `src/usage.rs`.
2. In `src/lib.rs`: `mod <thing>;`, add `"/<thing>"` to `OWNER`, and an arm in `owner_route`.
   That is all it takes to put it behind the token. Never check the token in the module itself.
3. Only if strangers must call it (as apps post `/usage`), widen `public()` for that one method
   and path, give it a rate limiter in `wrangler.toml` (a new `namespace_id`), and check size and
   shape before touching storage.
4. A new Durable Object class gets a binding and a new `[[migrations]]` tag in `wrangler.toml`.
   Never edit an old tag.
5. Unit tests in the module for anything pure, a phase in `smoke.mjs`, and a subcommand in
   `scripts/dev-endpoint` with both completions (`scripts/completions/_dev-endpoint` and
   `scripts/completions/dev-endpoint.bash`) in the same change.
6. Update the route table in `relay/dev/README.md` and this file.

Then `cargo test`, `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings` and
`./smoke.sh` in `relay/dev`, each through `buildq --local --`. Deploying is the owner's call:
`npx wrangler@4.161.0 deploy` in `relay/dev`.
