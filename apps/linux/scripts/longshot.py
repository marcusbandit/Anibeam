#!/usr/bin/env python3
"""longshot.py: one tall PNG of a whole scrolling page in the open Anibeam window.

For debugging. It drives the window through `anibeam ctl`: finds the largest scrolling view
on screen (or the one named with --target), steps its contentY one viewport at a time and
shoots each step. The view is put back where it was afterwards. It writes three files:

  <out>-window.png   the window exactly as it was on screen
  <out>-scroll.png   the scrolling area alone, stitched to its full content height
  <out>.png          both side by side on a grey sheet, each under a label, so nobody
                     reads the stitched part as the app's real layout

usage: longshot.py [out.png] [--root DIR] [--target OBJECTNAME] [--settle MS]
                   [--tiles PX] [--anibeam BIN]

  out.png         where the image goes (default: a file in $TMPDIR, printed at the end)
  --root DIR      the window started with that --root, as anibeam ctl takes it
  --target NAME   the objectName of the Flickable to scroll, instead of the largest one
  --settle MS     how long each step waits for images before the shot (default 400)
  --tiles PX      also cut <out>-scroll.png into <out>-scroll-01.png ... of PX height
  --anibeam BIN   the anibeam binary whose ctl to use (default: Anibeam Dev, then PATH)
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image, ImageDraw, ImageFont

# Finds the view and leaves it in `v`. Every eval reruns it, since nothing an eval makes
# outlives the request. The largest visible Flickable whose content is taller than itself wins,
# but only one nothing covers: the topmost painted item at its centre has to be inside it, so
# a page under a panel (onboarding, a drawer) is passed over for whatever is on top.
FIND = r"""
var v = null;
(function () {
  var want = %(target)s, best = null, area = 0, root = window.contentItem.parent || window.contentItem;
  function paints(it) {
    return (it.color !== undefined && it.color.a > 0) || it.source !== undefined
        || it.text !== undefined;  // a MouseArea draws nothing, so it covers nothing
  }
  function topAt(node, x, y) {
    var kids = Array.prototype.slice.call(node.children).map(function (c, i) { return [c, i]; });
    kids.sort(function (a, b) { return (b[0].z - a[0].z) || (b[1] - a[1]); });
    for (var i = 0; i < kids.length; i++) {
      var c = kids[i][0];
      if (!c.visible || c.opacity === 0) continue;
      var q = c.mapFromItem(root, x, y);
      if (q.x < 0 || q.y < 0 || q.x >= c.width || q.y >= c.height) continue;
      var hit = topAt(c, x, y);
      if (hit) return hit;
      if (paints(c)) return c;
    }
    return null;
  }
  function uncovered(it) {
    var c = it.mapToItem(root, it.width / 2, it.height / 2);
    for (var h = topAt(root, c.x, c.y); h; h = h.parent) if (h === it) return true;
    return false;
  }
  function walk(it) {
    if (!it || !it.visible || it.opacity === 0) return;
    if (it.contentY !== undefined && it.contentHeight !== undefined && it.flickableDirection !== undefined) {
      if (want !== null) { if (it.objectName === want) { best = it; area = Infinity; } }
      else if (it.contentHeight > it.height + 1 && it.width * it.height > area && uncovered(it)) {
        best = it; area = it.width * it.height;
      }
    }
    var c = it.children;
    for (var i = 0; i < c.length; i++) walk(c[i]);
  }
  walk(window.contentItem);
  v = best;
})();
"""

INFO = FIND + r"""
(function () {
  if (!v) return null;
  var p = v.mapToItem(null, 0, 0);
  return {x: p.x, y: p.y, w: v.width, h: v.height, y0: v.originY, ch: v.contentHeight,
          cy: v.contentY, ww: window.width, wh: window.height, name: v.objectName,
          kind: String(v).split("(")[0]};
})()
"""

MOVE = FIND + r"""
(function () {
  v.cancelFlick();
  v.contentY = %(y)s;
  return v.contentY;
})()
"""

# Where the view is now. A ListView with rows of mixed height only estimates contentHeight
# and moves originY as rows get built, so this is read again at every step.
WHERE = FIND + r"""
(function () { return {cy: v.contentY, y0: v.originY, ch: v.contentHeight, h: v.height}; })()
"""

MAX_STEPS = 400

# True once no image inside the view is still loading.
LOADED = FIND + r"""
(function () {
  var busy = 0;
  function walk(it) {
    if (!it || !it.visible) return;
    if (it.status === 2 && it.progress !== undefined && it.sourceSize !== undefined) busy++;
    var c = it.children;
    for (var i = 0; i < c.length; i++) walk(c[i]);
  }
  walk(v.contentItem);
  return busy === 0;
})()
"""


def anibeam_bin(given):
    if given:
        return given
    dev = os.path.expanduser("~/.local/opt/anibeam-dev/anibeam")
    if os.access(dev, os.X_OK):
        return dev
    found = shutil.which("anibeam")
    if not found:
        sys.exit("longshot: no anibeam binary; pass --anibeam")
    return found


class Ctl:
    def __init__(self, binary, root):
        self.base = [binary, "ctl"] + (["--root", root] if root else [])

    def run(self, *args, check=True):
        r = subprocess.run(self.base + list(args), capture_output=True, text=True)
        if check and r.returncode != 0:
            msg = r.stderr.strip() or r.stdout.strip()
            sys.exit(f"longshot: anibeam ctl {args[0]} failed ({r.returncode}): {msg}")
        return r

    def eval(self, js):
        out = self.run("eval", js).stdout.strip()
        return json.loads(out) if out else None


def settle_still(ctl, sub):
    """Waits for a running glide (WheelStep's contentY animation) to end before stepping."""
    last = None
    for _ in range(40):
        now = ctl.eval(WHERE % sub)["cy"]
        if now == last:
            return
        last = now
        time.sleep(0.05)


def shoot(ctl, sub, info, work, settle_ms):
    """Steps the view from the top, one viewport at a time, until it reaches the current end.
    Returns [(offset into the content, image)], the stitched scroll image and the scale."""
    shots, pos = [], info["y0"]
    try:
        for i in range(MAX_STEPS):
            ctl.eval(MOVE % dict(sub, y=pos))
            time.sleep(settle_ms / 1000)
            ctl.run("wait", LOADED % sub, "--timeout", "3000", check=False)
            f = os.path.join(work, f"{i:03}.png")
            ctl.run("shot", f)
            at = ctl.eval(WHERE % sub)
            shots.append((at["cy"] - at["y0"], Image.open(f).convert("RGB")))
            end = at["y0"] + max(0.0, at["ch"] - at["h"])
            if at["cy"] >= end - 0.5:
                break
            pos = min(at["cy"] + at["h"], end)
        else:
            print(f"# stopped after {MAX_STEPS} steps", file=sys.stderr)
    finally:
        ctl.run("eval", MOVE % dict(sub, y=info["cy"]), check=False)

    win = shots[0][1]
    s = win.width / info["ww"]  # device pixels per window pixel
    vx, vy = round(info["x"] * s), round(info["y"] * s)
    vw, h = round(info["w"] * s), info["h"]
    bottom = min(win.height, round((info["y"] + h) * s))  # the view may reach past the window
    full = Image.new("RGB", (vw, round((max(o for o, _ in shots) + h) * s)))
    for off, img in shots:
        top = round(off * s)
        rows = min(round((off + h) * s) - top, bottom - vy)  # each tile meets the next exactly
        full.paste(img.crop((vx, vy, vx + vw, vy + rows)), (0, top))
    return shots, full, s


def font(size):
    try:
        return ImageFont.load_default(size=size)  # Pillow 10.1+ with FreeType
    except (TypeError, OSError):
        return ImageFont.load_default()


def main():
    ap = argparse.ArgumentParser(add_help=True, usage=__doc__)
    ap.add_argument("out", nargs="?")
    ap.add_argument("--root")
    ap.add_argument("--target")
    ap.add_argument("--settle", type=int, default=400)
    ap.add_argument("--tiles", type=int, default=0)
    ap.add_argument("--anibeam")
    a = ap.parse_args()

    out = os.path.abspath(a.out or os.path.join(tempfile.gettempdir(), f"anibeam-longshot-{int(time.time())}.png"))
    if not out.endswith(".png"):
        out += ".png"
    ctl = Ctl(anibeam_bin(a.anibeam), a.root)
    sub = {"target": json.dumps(a.target)}

    info = ctl.eval(INFO % sub)
    if not info:
        window_png = out[:-4] + "-window.png"
        ctl.run("shot", window_png)
        print("# no scrolling view on screen" + (f" named {a.target}" if a.target else "")
              + "; only the window shot was taken", file=sys.stderr)
        print(window_png)
        return
    if info["h"] < 1:
        sys.exit(f"longshot: the view is {info['h']:.0f} px tall; nothing to step through")

    work = tempfile.mkdtemp(prefix="longshot-")
    try:
        settle_still(ctl, sub)
        info = ctl.eval(INFO % sub)  # where the view rests, to put it back there
        ctl.run("shot", os.path.join(work, "window.png"))  # before anything moves
        first = Image.open(os.path.join(work, "window.png")).convert("RGB")
        shots, full, s = shoot(ctl, sub, info, work, a.settle)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    stem = out[:-4]
    window_png, scroll_png = f"{stem}-window.png", f"{stem}-scroll.png"
    first.save(window_png)
    full.save(scroll_png)

    name = info["name"] or info["kind"]
    labels = [
        ("1  WINDOW, AS SHOWN", "static, nothing stitched"),
        ("2  SCROLLING AREA ONLY, FULL LENGTH", f"{len(shots)} shots stitched, {name}"),
    ]
    big, small = font(max(18, round(22 * s))), font(max(14, round(15 * s)))
    pad, band = round(24 * s), round(72 * s)
    probe = ImageDraw.Draw(Image.new("RGB", (1, 1)))
    cols = []
    for (title, detail), img in zip(labels, (first, full)):
        text_w = max(probe.textlength(title, font=big), probe.textlength(detail, font=small))
        cols.append((title, detail, img, max(img.width, round(text_w))))
    w = pad + sum(c[3] + pad for c in cols)
    h = band + max(first.height, full.height) + pad
    sheet = Image.new("RGB", (w, h), (128, 128, 128))
    draw = ImageDraw.Draw(sheet)
    x = pad
    for title, detail, img, col_w in cols:
        draw.text((x, round(band * 0.18)), title, fill=(255, 255, 255), font=big)
        draw.text((x, round(band * 0.6)), detail, fill=(235, 235, 235), font=small)
        sheet.paste(img, (x, band))
        x += col_w + pad
    sheet.save(out)

    lines = [out, window_png, scroll_png]
    if a.tiles:
        for n, top in enumerate(range(0, full.height, a.tiles), 1):
            t = f"{stem}-scroll-{n:02}.png"
            full.crop((0, top, full.width, min(top + a.tiles, full.height))).save(t)
            lines.append(t)
    print(f"# {name}: {len(shots)} steps, scroll image {full.width}x{full.height}", file=sys.stderr)
    print("\n".join(lines))


if __name__ == "__main__":
    main()
