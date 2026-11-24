#!/usr/bin/env bash
# Fetch every Lucide glyph the shell names into assets/icons/, stroke set to black so QtSvg
# reads it and ColorImage tints it. Re-run to add a name; then list it in build.rs.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
tag="1.41.0"
out="$here/assets/icons"; mkdir -p "$out"
names=(
  activity archive arrow-down arrow-down-wide-narrow arrow-left arrow-right arrow-up arrow-up-narrow-wide arrow-up-right
  audio-lines badge-check ban bell book-open bookmark bug calendar calendar-clock captions case-sensitive cast chart-pie
  check check-check chevron-down chevron-left chevron-right chevron-up chevrons-right circle-alert circle-check circle-play
  circle-question-mark circle-x clapperboard clock copy crown database download ellipsis external-link eye eye-off
  file-down file-up film folder folder-open folder-plus folder-search folder-x gauge git-branch globe hard-drive heart
  house image info keyboard languages layers layout-grid link list list-filter list-video loader-circle lock log-in
  log-out maximize memory-stick minimize minus monitor moon palette pause pencil play plus radio recycle refresh-cw
  rotate-ccw rotate-cw rss scan scan-search search settings shapes skip-back skip-forward sliders-horizontal sparkles
  square square-check star step-back step-forward sun thumbs-up timer toggle-right trash trash-2 triangle-alert tv type
  upload user-check users volume-2 volume-x wrench x
)
# Fetched to a temporary file and moved into place only on success: a 404 or a dropped
# connection must not leave a zero-byte SVG behind, because a blank icon is not a build error.
for n in "${names[@]}"; do
  [ -f "$out/$n.svg" ] && continue
  tmp="$(mktemp "$out/.$n.XXXXXX")"
  if curl -fsSL "https://raw.githubusercontent.com/lucide-icons/lucide/$tag/icons/$n.svg" \
      | sed 's/stroke="currentColor"/stroke="#000"/' > "$tmp" && [ -s "$tmp" ]; then
    chmod 644 "$tmp"
    mv "$tmp" "$out/$n.svg"
  else
    rm -f "$tmp"
    echo "no glyph named $n at lucide $tag" >&2
  fi
done
ls "$out"/*.svg | wc -l
