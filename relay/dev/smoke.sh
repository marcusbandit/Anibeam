#!/usr/bin/env bash
# Runs the dev Worker locally under `wrangler dev` (local mode, no Cloudflare
# login) with a read token made for the run, and drives it twice: smoke.mjs
# checks the routes, then scripts/dev-endpoint reads, pulls, sums and prunes
# what smoke.mjs left behind. Nothing leaves the machine.
#
#   ./smoke.sh
#
# Needs node 22 or later, python 3, worker-build on PATH and the
# wasm32-unknown-unknown target. PORT picks the local port (default 8796).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
port="${PORT:-8796}"
base="http://127.0.0.1:${port}"
wrangler="${WRANGLER:-npx --yes wrangler@4.161.0}"
work="$(mktemp -d)"
pid=""
# The owner's token for this run only, handed to wrangler as a var.
export SMOKE_DEV_TOKEN="smoke-$RANDOM-$RANDOM"

stop() {
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    pid=""
}
trap 'stop; rm -rf "$work"' EXIT

(cd "$here" && exec $wrangler dev --local --ip 127.0.0.1 --port "$port" \
    --persist-to "$work/state" --show-interactive-dev-session=false \
    --var "DEV_READ_TOKEN:$SMOKE_DEV_TOKEN") \
    >>"$work/wrangler.log" 2>&1 &
pid=$!
up=""
for _ in $(seq 1 300); do
    # There is no public GET route; any answer at all means it is up.
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "$base/" || true)" == "404" ]]; then
        up=1
        break
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
        break
    fi
    sleep 1
done
if [[ -z "$up" ]]; then
    echo "wrangler dev did not come up; its log:" >&2
    cat "$work/wrangler.log" >&2
    exit 1
fi

node "$here/smoke.mjs" "$base"

# The reading script, against the same local Worker, into a throwaway folder.
script="$repo/scripts/dev-endpoint"
export ANIBEAM_DEV_ENDPOINT="$base" ANIBEAM_DEV_TOKEN="$SMOKE_DEV_TOKEN" XDG_DATA_HOME="$work/data"
fail=0
check() {
    if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}
list="$("$script" usage list)"
echo "$list" | sed 's/^/     /'
check "usage list shows the one send smoke.mjs kept" '[[ "$(grep -c "0.0.0-smoke" <<<"$list")" == 1 ]]'
pulled="$("$script" usage pull)"
check "usage pull saves it ($pulled)" '[[ "$(ls "$work/data/anibeam/usage-sends/"*.json | wc -l)" == 1 ]]'
again="$("$script" usage pull)"
check "a second pull finds nothing new ($again)" '[[ "$again" == *"no new sends"* ]]'
summary="$("$script" usage summary)"
echo "$summary" | sed 's/^/     /'
check "usage summary sums its calls" 'grep -Eq "Scan +4" <<<"$summary"'
check "no token is a clear refusal" '! ANIBEAM_DEV_TOKEN= XDG_DATA_HOME= "$script" --no-keyring usage list 2>&1 | grep -q Traceback'
pruned="$("$script" usage prune --before 2999-01-01)"
check "usage prune drops it ($pruned)" '[[ "$pruned" == *"1 send"* ]]'
check "and then the list is empty" '[[ -z "$("$script" usage list)" ]]'

if [[ "$fail" != 0 ]]; then
    echo "smoke: the script checks failed" >&2
    exit 1
fi
echo "smoke: all phases passed"
