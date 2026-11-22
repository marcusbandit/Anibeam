#!/usr/bin/env python3
"""Builds the website's wiki (#1007): clear, instructional guides the app can link to.

  <out>/wiki/index.html            guide hub with search and grouped cards
  <out>/wiki/<slug>/index.html     one guide page per entry in PAGES

Each page's content is an HTML fragment in site/wiki/<slug>.html: sections with an <h2 id="...">
so links such as /wiki/sources/#add land directly on that step. This script wraps each fragment
in the site's nav and footer, adds a grouped guide sidebar, an auto-generated "On this page"
outline, section anchor links, and a Previous/Next pager. Slugs are stable addresses the app
links to, so a slug never changes once shipped.

The wiki is English only, like What's new, so its language menu leads to each language's home.

usage: wiki.py <repo-root> <out-dir>
"""
import html
import json
import re
import shutil
import sys
from pathlib import Path

root, out = Path(sys.argv[1]), Path(sys.argv[2])
SRC = root / "site/wiki"

GROUPS = [
    ("start", "Start here"),
    ("core", "Tracking and downloads"),
    ("more", "Watch together and phone"),
    ("help", "Help"),
]

# slug, group, title, lede (shown under h1), summary (cards & meta description), icon (Lucide SVG paths), quick topics
PAGES = [
    (
        "getting-started",
        "start",
        "Getting started",
        "Everything you need to install Anibeam, breeze through the one-minute setup, and find your way around the app.",
        "Install Anibeam, finish the quick first-time setup, and learn what each page does.",
        '<path d="M6 3l14 9-14 9z"/>',
        [("install", "Install"), ("setup", "First-time setup"), ("find-your-way", "App tour")],
    ),
    (
        "library",
        "start",
        "Your library",
        "Point Anibeam at your anime folders, name files so they match cleanly, and fix anything Anibeam misreads.",
        "Add your anime folders, name files so Anibeam recognizes them, and fix mismatches.",
        '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
        [("add", "Add folders"), ("naming", "Folder naming"), ("match", "Fix matches")],
    ),
    (
        "playback",
        "start",
        "Watching and casting",
        "Get the most out of the built-in player: keyboard shortcuts, automatic intro skipping, subtitle styling, TV casting, and Discord.",
        "Use the built-in player, customize subtitles, skip intros, and cast to your TV.",
        '<rect width="20" height="15" x="2" y="4" rx="2"/><path d="M8 21h8"/><path d="M12 19v2"/><path d="m10 9 5 3-5 3z"/>',
        [("shortcuts", "Shortcuts"), ("skip", "Skip intros"), ("cast", "Cast to TV")],
    ),
    (
        "tracking",
        "core",
        "AniList and MyAnimeList",
        "Sign in once in your browser so every episode you finish updates your anime list automatically, with no manual logging.",
        "Connect AniList or MyAnimeList to sync watched episodes, ratings, lists, and notes.",
        '<path d="M20 6L9 17l-5-5"/>',
        [("anilist", "AniList"), ("myanimelist", "MyAnimeList"), ("both", "Use both")],
    ),
    (
        "sources",
        "core",
        "Download sources",
        "Anibeam starts empty with no download sites built in. Add a community source list or plug in your own RSS or Torznab feed directly.",
        "Add a source list or your own RSS/Torznab feed directly, and see what format Anibeam expects.",
        '<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/>',
        [("lists", "Source lists"), ("by-hand", "Add directly"), ("updates", "Updates")],
    ),
    (
        "downloads",
        "core",
        "Downloads",
        "Download episodes with Anibeam's built-in torrent client or your own qBittorrent, paste magnet links, follow airing shows, and save disk space.",
        "Get episodes, paste magnet links, follow airing shows, and manage disk space.",
        '<path d="M12 3v12"/><path d="M7 10l5 5 5-5"/><path d="M4 21h16"/>',
        [("magnets", "Magnet links"), ("follow", "Follow shows"), ("long", "Rolling")],
    ),
    (
        "watch-together",
        "more",
        "Watch together",
        "Watch anime in sync with up to 10 friends at full quality, share a room with a code or link, and fetch missing episodes right from the room.",
        "Start a synced room, invite friends with a code, QR code, or Discord, and stay in step.",
        '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
        [("start", "Start a room"), ("join", "Join a room"), ("get-media", "Missing episodes"), ("offset", "Sync offset")],
    ),
    (
        "phone",
        "more",
        "Phone app and streaming",
        "Install Anibeam on Android or iPhone to watch on the go, or connect to your computer over Tailscale to stream your desktop library from anywhere.",
        "Use Anibeam on Android or iPhone, or stream your computer's library over Tailscale.",
        '<rect width="12" height="20" x="6" y="2" rx="2"/><path d="M12 18h.01"/>',
        [("install", "Android & iPhone"), ("tailscale", "Tailscale"), ("streaming", "Streaming"), ("phone-downloads", "Phone downloads")],
    ),
    (
        "troubleshooting",
        "help",
        "Troubleshooting",
        "Plain-English fixes for the most common snags: first-launch warnings, missing shows, downloads, sign-in, casting, rooms, and backups.",
        "Stuck on something? Jump straight to a plain-English fix for install warnings, missing shows, downloads, casting, or Tailscale.",
        '<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.5-.5-.5-2.5z"/>',
        [
            ("install-warnings", "Install warnings"),
            ("library-empty", "Empty library"),
            ("not-recognised", "Wrong show match"),
            ("no-sources", "No sources"),
            ("not-arriving", "Downloads stuck"),
            ("playback-issues", "Playback & casting"),
            ("tailscale-issues", "Tailscale issues"),
            ("data", "Backups & data"),
        ],
    ),
]

# The nav and the footer come from the download page, as on What's new: it sits at the same
# depth as the wiki's index, and a page one level down climbs one more.
_dl = (out / "download/index.html").read_text()
NAV = re.search(r'<header class="nav">.*?</header>', _dl, re.S).group(0)
FOOT = re.search(r'<footer class="credits">.*?</footer>', _dl, re.S).group(0)
# The language menu leads to each language's home page, since the wiki is English only.
NAV = re.sub(r'(<a href=")\.\./download/(" hreflang="en"[^>]*aria-current="true")', r'\1../wiki/\2', NAV)
NAV = re.sub(
    r'(<a href=")\.\./(?:([\w-]+)/)?download/(" hreflang=)',
    lambda m: f'{m.group(1)}../{m.group(2) + "/" if m.group(2) else ""}{m.group(3)}',
    NAV,
)
WIKI_LINK = '<a class="nav-wiki" href="../wiki/">'
if WIKI_LINK not in NAV:
    sys.exit("wiki: the download page's nav has no Wiki link to mark as current")
NAV = NAV.replace(WIKI_LINK, '<a class="nav-wiki" href="../wiki/" aria-current="page">', 1)

H2_RE = re.compile(r'<h2\b([^>]*)>(.*?)</h2>', re.S)
TAG_RE = re.compile(r'<[^>]+>')


def chrome(fragment, prefix):
    if prefix == "../":
        return fragment
    return re.sub(r'(href|src)="\.\./', lambda m: f'{m.group(1)}="{prefix}', fragment)


def icon(paths):
    return f'<svg viewBox="0 0 24 24" aria-hidden="true">{paths}</svg>'


def page(title, prefix, body, canonical, description, main_class="wiki"):
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(description)}">
<link rel="canonical" href="https://anibeam.app/{canonical}">
<meta name="theme-color" content="#07080a">
<meta name="color-scheme" content="dark">
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="{prefix}img/logo.svg" type="image/svg+xml">
<meta property="og:site_name" content="Anibeam">
<meta property="og:type" content="article">
<meta property="og:title" content="{html.escape(title)}">
<meta property="og:description" content="{html.escape(description)}">
<meta property="og:url" content="https://anibeam.app/{canonical}">
<meta property="og:image" content="https://anibeam.app/img/og.jpg">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta name="twitter:card" content="summary_large_image">
<link rel="stylesheet" href="{prefix}style.css">
<link rel="stylesheet" href="{prefix}redesign.css">
<script src="{prefix}common.js" defer></script>
<script src="{prefix}lang.js" defer></script>
<script src="{prefix}redesign.js" defer></script>
<script src="{prefix}wiki.js" defer></script>
</head>
<body class="rd">
{chrome(NAV, prefix).replace(f'href="{prefix}wiki/" hreflang="en"', f'href="{prefix}{canonical}" hreflang="en"', 1)}
<main class="{main_class}">
{body}
</main>
{chrome(FOOT, prefix)}
</body>
</html>
"""


def check_and_prepare(slug, text):
    """Verify section ids and dash rules, extract headings for the outline, and add anchor links."""
    if re.search("[\u2013\u2014]", text):
        sys.exit(f"wiki: site/wiki/{slug}.html has an en or em dash")
    seen = set()
    headings = []

    def decorate(m):
        attrs, inner = m.group(1), m.group(2)
        found = re.search(r'(?:^|\s)id="([\w-]+)"', attrs)
        if not found:
            sys.exit(f"wiki: site/wiki/{slug}.html has an <h2> without an id")
        hid = found.group(1)
        if hid in seen:
            sys.exit(f"wiki: site/wiki/{slug}.html has two sections with the id {hid}")
        seen.add(hid)
        label = html.unescape(TAG_RE.sub("", inner)).strip()
        headings.append((hid, label))
        anchor = f'<a class="wiki-anchor" href="#{hid}" aria-label="Link to {html.escape(label, quote=True)}">#</a>'
        return f"<h2{attrs}>{inner}{anchor}</h2>"

    rendered = H2_RE.sub(decorate, text)
    return rendered, headings


fragments = {}
sections_by_slug = {}
group_labels = dict(GROUPS)

for slug, group, title, lede, summary, _, topics in PAGES:
    if group not in group_labels:
        sys.exit(f"wiki: unknown group {group!r} for {slug}")
    for field in (title, lede, summary, *(label for _, label in topics)):
        if re.search("[\u2013\u2014]", field):
            sys.exit(f"wiki: metadata for {slug} has an en or em dash")
    text = (SRC / f"{slug}.html").read_text()
    rendered, headings = check_and_prepare(slug, text)
    heading_ids = {hid for hid, _ in headings}
    for tid, _ in topics:
        if tid not in heading_ids:
            sys.exit(f"wiki: topic anchor #{tid} does not exist in site/wiki/{slug}.html")
    fragments[slug] = rendered
    sections_by_slug[slug] = headings

extra = sorted(p.stem for p in SRC.glob("*.html") if p.stem not in fragments)
if extra:
    sys.exit(f"wiki: site/wiki/ has pages PAGES does not list: {', '.join(extra)}")

# The fragments were copied in with the rest of site/; the built pages replace them.
shutil.rmtree(out / "wiki", ignore_errors=True)
(out / "wiki").mkdir(parents=True)


def render_sidebar(current_slug):
    blocks = []
    for gkey, glabel in GROUPS:
        links = []
        for s, g, t, _, _, paths, _ in PAGES:
            if g != gkey:
                continue
            cur = ' aria-current="page"' if s == current_slug else ""
            links.append(
                f'<a href="../{s}/" data-wiki-slug="{s}"{cur}>{icon(paths)}<span>{html.escape(t)}</span></a>'
            )
        blocks.append(
            f'<div class="wiki-nav-group">'
            f'<span class="wiki-nav-label">{html.escape(glabel)}</span>'
            f'{"".join(links)}'
            f"</div>"
        )
    return "".join(blocks)


# ---- Wiki index (/wiki/)
index_groups = []
for gkey, glabel in GROUPS:
    group_pages = [p for p in PAGES if p[1] == gkey]
    if gkey == "help" and len(group_pages) == 1:
        slug, _, title, _, summary, paths, topics = group_pages[0]
        topic_links = "".join(
            f'<a href="{slug}/#{tid}">{html.escape(tlabel)}</a>' for tid, tlabel in topics
        )
        index_groups.append(
            f'<section class="wiki-group" aria-labelledby="wg-{gkey}">\n'
            f'  <h2 class="wiki-group-title" id="wg-{gkey}">Need a fix?</h2>\n'
            f'  <article class="wiki-help-card">\n'
            f'    <div class="wiki-help-head">\n'
            f'      <a class="wiki-card-link wiki-help-main" href="{slug}/">\n'
            f'        <span class="wiki-card-ic">{icon(paths)}</span>\n'
            f'        <div><h3>{html.escape(title)}</h3><p>{html.escape(summary)}</p></div>\n'
            f'      </a>\n'
            f'      <a class="wiki-help-cta" href="{slug}/">All fixes <span aria-hidden="true">&#8594;</span></a>\n'
            f'    </div>\n'
            f'    <div class="wiki-card-topics">{topic_links}</div>\n'
            f'  </article>\n'
            f"</section>"
        )
        continue

    cards = []
    for idx, (slug, _, title, _, summary, paths, topics) in enumerate(group_pages, start=1):
        step_badge = f'<span class="wiki-card-step">0{idx}</span>' if gkey == "start" else ""
        topic_links = "".join(
            f'<a href="{slug}/#{tid}">{html.escape(tlabel)}</a>' for tid, tlabel in topics
        )
        cards.append(
            f'    <article class="wiki-card">\n'
            f'      <a class="wiki-card-link" href="{slug}/">\n'
            f'        <div class="wiki-card-top">'
            f'<div class="wiki-card-badges"><span class="wiki-card-ic">{icon(paths)}</span>{step_badge}</div>'
            f'<span class="wiki-card-arrow" aria-hidden="true">&#8594;</span></div>\n'
            f'        <h3>{html.escape(title)}</h3>\n'
            f'        <p>{html.escape(summary)}</p>\n'
            f'      </a>\n'
            f'      <div class="wiki-card-topics">{topic_links}</div>\n'
            f'    </article>'
        )
    grid_cls = "wiki-cards wiki-cards-2" if len(group_pages) == 2 else "wiki-cards"
    index_groups.append(
        f'<section class="wiki-group" aria-labelledby="wg-{gkey}">\n'
        f'  <h2 class="wiki-group-title" id="wg-{gkey}">{html.escape(glabel)}</h2>\n'
        f'  <div class="{grid_cls}">\n' + "\n".join(cards) + "\n  </div>\n</section>"
    )

search_data = [
    {
        "slug": slug,
        "title": title,
        "group": group_labels[group],
        "summary": summary,
        "sections": [{"id": sid, "title": stitle} for sid, stitle in sections_by_slug[slug]],
    }
    for slug, group, title, _, summary, _, _ in PAGES
]
search_json = json.dumps(search_data, ensure_ascii=False).replace("</", "<\\/")

index_body = f"""<section class="wiki-index-hero">
  <div class="wiki-index-head">
    <h1>Anibeam Wiki</h1>
    <p class="wiki-lede">Plain-English guides for setting up your library, watching, syncing anime lists, downloading episodes, and watching with friends.</p>
  </div>
  <div class="wiki-search" role="search">
    <svg class="wiki-search-ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
    <input type="search" id="wiki-search-input" placeholder="Search guides, settings, or fixes..." aria-label="Search the Anibeam wiki" autocomplete="off">
    <button type="button" id="wiki-search-clear" class="wiki-search-clear" hidden aria-label="Clear search">Clear</button>
  </div>
  <div id="wiki-search-results" class="wiki-search-results" hidden aria-live="polite"></div>
</section>
<div id="wiki-groups" class="wiki-groups">
{chr(10).join(index_groups)}
</div>
<script id="wiki-index-data" type="application/json">{search_json}</script>"""

(out / "wiki/index.html").write_text(
    page(
        "Anibeam Wiki",
        "../",
        index_body,
        "wiki/",
        "Plain-English guides for setting up and using Anibeam: library, player, AniList and MyAnimeList, sources, downloads, Watch together, phone app, and troubleshooting.",
        main_class="wiki wiki-index",
    )
)

# ---- Individual guide pages (/wiki/<slug>/)
for i, (slug, group, title, lede, summary, _, _) in enumerate(PAGES):
    sidebar_html = render_sidebar(slug)
    headings = sections_by_slug[slug]
    onpage_links = "".join(
        f'<a href="#{hid}"{" class=\"is-active\"" if idx == 0 else ""}>{html.escape(label)}</a>'
        for idx, (hid, label) in enumerate(headings)
    )
    onpage_mobile_links = "".join(f'<a href="#{hid}">{html.escape(label)}</a>' for hid, label in headings)

    onpage_mobile = (
        f'<details class="wiki-jump">'
        f'<summary>On this page ({len(headings)} sections)</summary>'
        f'<nav aria-label="On this page">{onpage_mobile_links}</nav>'
        f"</details>"
        if headings
        else ""
    )
    onpage_desktop = (
        f'<aside class="wiki-onpage" aria-label="On this page"{"" if headings else " hidden"}>'
        f'<span class="wiki-onpage-title">On this page</span>'
        f'<nav>{onpage_links}</nav>'
        f"</aside>"
    )

    older = PAGES[i - 1] if i > 0 else None
    newer = PAGES[i + 1] if i + 1 < len(PAGES) else None
    prev_html = (
        f'<a class="wiki-pager-link wiki-prev" href="../{older[0]}/">'
        f'<span class="wiki-pager-dir">Previous</span>'
        f'<span class="wiki-pager-name">{html.escape(older[2])}</span></a>'
        if older
        else "<span></span>"
    )
    next_html = (
        f'<a class="wiki-pager-link wiki-next" href="../{newer[0]}/">'
        f'<span class="wiki-pager-dir">Next</span>'
        f'<span class="wiki-pager-name">{html.escape(newer[2])}</span></a>'
        if newer
        else "<span></span>"
    )
    pager = f'<nav class="wiki-pager" aria-label="Next and previous guides">{prev_html}{next_html}</nav>'

    body = f"""<div class="wiki-shell" data-wiki-slug="{slug}" data-wiki-title="{html.escape(title, quote=True)}">
  <aside class="wiki-sidebar">
    <nav class="wiki-nav" aria-label="Wiki guides">
      <a class="wiki-nav-home" href="../" data-wiki-home="1">&#8592; All guides</a>
      {sidebar_html}
    </nav>
    <details class="wiki-mobile-nav">
      <summary><span>All guides: <b>{html.escape(title)}</b></span></summary>
      <nav aria-label="Wiki guides">
        <a class="wiki-nav-home" href="../" data-wiki-home="1">&#8592; Wiki home</a>
        {sidebar_html}
      </nav>
    </details>
  </aside>
  <div class="wiki-main">
    <header class="wiki-article-head">
      <div class="wiki-crumb"><a href="../">Wiki</a><span aria-hidden="true">/</span><span>{html.escape(group_labels[group])}</span></div>
      <h1>{html.escape(title)}</h1>
      <p class="wiki-lede">{html.escape(lede)}</p>
    </header>
    {onpage_mobile}
    <article class="wiki-text">
{fragments[slug].strip()}
    </article>
    {pager}
  </div>
  {onpage_desktop}
</div>"""

    (out / "wiki" / slug).mkdir(parents=True, exist_ok=True)
    (out / "wiki" / slug / "index.html").write_text(
        page(f"{title}: Anibeam Wiki", "../../", body, f"wiki/{slug}/", summary)
    )

# The wiki goes in the sitemap next to the pages i18n.py listed.
sitemap = out / "sitemap.xml"
if sitemap.exists():
    urls = "".join(
        f"  <url><loc>https://anibeam.app/wiki/{s}</loc></url>\n"
        for s in [""] + [f"{p[0]}/" for p in PAGES]
    )
    sitemap.write_text(sitemap.read_text().replace("</urlset>", urls + "</urlset>"))

print(f"wiki: {len(PAGES)} pages")
