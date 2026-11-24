#!/usr/bin/env bash
# usage: discord-entry.sh <anibeam binary> <applications dir>
# Writes discord-<id>.desktop, the hidden entry that handles discord-<id>:// links, into
# <applications dir>, so Join from Discord starts a closed Anibeam on a package install
# (#1167). The id is ANIBEAM_DISCORD_CLIENT_ID, read the way the build reads it (playback/build.rs):
# the environment first, then the checkout's .env.local. Without one the build has no Discord
# application and nothing is written. The binary must carry the id, or a package would claim
# links for an application it never talks to.
# install-tree.sh (the AUR's anibeam and anibeam-bin) and packages.sh (the deb and rpm) call it.
# The app writes the same entry into the user's folder for an AppImage (src/discord_link.rs).
set -euo pipefail
bin="${1:?usage: discord-entry.sh <binary> <applications dir>}"
dir="${2:?usage: discord-entry.sh <binary> <applications dir>}"
trim() { local v="$1"; v="${v#"${v%%[![:space:]]*}"}"; printf '%s' "${v%"${v##*[![:space:]]}"}"; }
# One KEY=VALUE line's value, read as trackers/env_local.rs's dotenv_value reads it: comments
# and blank lines skipped, an export prefix allowed, one pair of matching quotes stripped,
# else a " #" comment dropped, the last line naming the key winning.
dotenv() {
  local line key value found=""
  while IFS= read -r line || [ -n "$line" ]; do
    line="$(trim "$line")"
    [ -z "$line" ] || [ "${line:0:1}" = "#" ] && continue
    line="${line#export }"
    [[ "$line" == *=* ]] || continue
    key="$(trim "${line%%=*}")"
    [ "$key" = "$2" ] || continue
    value="$(trim "${line#*=}")"
    if [[ "$value" =~ ^\"(.*)\"$ || "$value" =~ ^\'(.*)\'$ ]]; then
      value="${BASH_REMATCH[1]}"
    else
      value="${value%% #*}"
    fi
    found="$(trim "$value")"
  done < "$1"
  printf '%s' "$found"
}
id="$(trim "${ANIBEAM_DISCORD_CLIENT_ID:-}")"
local_env="$(cd "$(dirname "$0")/../../../.." && pwd)/.env.local"
if [ -z "$id" ] && [ -f "$local_env" ]; then
  id="$(dotenv "$local_env" ANIBEAM_DISCORD_CLIENT_ID)"
fi
[ -n "$id" ] || exit 0
if [[ ! "$id" =~ ^[0-9]{17,20}$ ]]; then
  echo "discord-entry.sh: ANIBEAM_DISCORD_CLIENT_ID is not a Discord application id" >&2
  exit 1
fi
if ! grep -qaF -- "$id" "$bin"; then
  echo "discord-entry.sh: $bin was built without Discord application $id" >&2
  exit 1
fi
install -d "$dir"
printf '%s\n' \
  '[Desktop Entry]' \
  'Type=Application' \
  'Name=Anibeam' \
  'Exec=anibeam %u' \
  'NoDisplay=true' \
  'Terminal=false' \
  "MimeType=x-scheme-handler/discord-$id;" > "$dir/discord-$id.desktop"
chmod 644 "$dir/discord-$id.desktop"
