// Progressive enhancements for the Anibeam Wiki:
// 1. In-place client-side navigation between wiki pages (sidebar, header, and footer stay put;
//    only the article content and "On this page" outline swap, with prefetching and history).
// 2. Instant topic search on /wiki/ across guide titles, summaries, and section headings.
// 3. Active-section highlighting in the sticky "On this page" sidebar on guide pages.
(() => {
  const canSoftNav = location.protocol === "http:" || location.protocol === "https:";
  const scriptSrc = document.currentScript && document.currentScript.src;
  const siteRoot =
    (window.AB && AB.ROOT) ||
    (scriptSrc ? new URL("./", scriptSrc).href : new URL("/", location.href).href);
  const wikiRoot = new URL("wiki/", siteRoot);
  const wikiOrigin = wikiRoot.origin;
  const wikiPrefix = wikiRoot.pathname; // ends with "/wiki/"

  const esc = (s) =>
    s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

  // Normalize any same-origin URL inside the wiki to a canonical route descriptor, or null.
  function parseWikiRoute(rawUrl, base = location.href) {
    let url;
    try {
      url = new URL(rawUrl, base);
    } catch {
      return null;
    }
    if (url.origin !== wikiOrigin) return null;
    if (!url.pathname.startsWith(wikiPrefix)) return null;

    let rest = url.pathname.slice(wikiPrefix.length);
    rest = rest.replace(/(?:^|\/)index\.html$/, "").replace(/^\/+|\/+$/g, "");
    if (rest === "") {
      return {
        kind: "index",
        slug: "",
        key: wikiPrefix,
        href: wikiPrefix + url.search + url.hash,
        hash: url.hash,
      };
    }
    if (rest.includes("/") || rest.includes(".")) return null;
    const key = `${wikiPrefix}${rest}/`;
    return {
      kind: "guide",
      slug: rest,
      key,
      href: key + url.search + url.hash,
      hash: url.hash,
    };
  }

  // Pin relative href/src attributes in a subtree against `baseUrl` so they keep resolving
  // cleanly even after history.pushState moves between /wiki/ and /wiki/<slug>/.
  function pinRelativeUrls(rootEl, baseUrl) {
    if (!rootEl) return;
    for (const el of rootEl.querySelectorAll("[href], [src]")) {
      for (const attr of ["href", "src"]) {
        const raw = el.getAttribute(attr);
        if (
          !raw ||
          raw.startsWith("#") ||
          raw.startsWith("/") ||
          /^[a-z][a-z0-9+.-]*:/i.test(raw)
        ) {
          continue;
        }
        try {
          const resolved = new URL(raw, baseUrl);
          if (resolved.origin === location.origin) {
            el.setAttribute(attr, resolved.pathname + resolved.search + resolved.hash);
          }
        } catch {
          // Leave malformed URLs untouched.
        }
      }
    }
  }

  // ---- Wiki index search (/wiki/)
  function initSearch() {
    const input = document.getElementById("wiki-search-input");
    const dataEl = document.getElementById("wiki-index-data");
    const resultsEl = document.getElementById("wiki-search-results");
    const groupsEl = document.getElementById("wiki-groups");
    const clearBtn = document.getElementById("wiki-search-clear");

    if (!input || !dataEl || !resultsEl || !groupsEl) return;

    let index = [];
    try {
      index = JSON.parse(dataEl.textContent || "[]");
    } catch {
      index = [];
    }

    function render(raw) {
      const q = raw.trim().toLowerCase();
      if (clearBtn) clearBtn.hidden = q === "";
      if (!q) {
        resultsEl.hidden = true;
        resultsEl.innerHTML = "";
        groupsEl.hidden = false;
        return;
      }

      const terms = q.split(/\s+/).filter(Boolean);
      const matches = [];
      for (const page of index) {
        const hay = `${page.title} ${page.summary} ${page.group}`.toLowerCase();
        const sectionHits = page.sections.filter((s) =>
          terms.every((t) => `${s.title} ${page.title}`.toLowerCase().includes(t))
        );
        const pageHit = terms.every((t) => hay.includes(t));
        if (pageHit || sectionHits.length > 0) {
          matches.push({ page, sectionHits });
        }
      }

      groupsEl.hidden = true;
      resultsEl.hidden = false;

      if (matches.length === 0) {
        resultsEl.innerHTML =
          `<p class="wiki-search-empty">No guides match <b>${esc(raw.trim())}</b>. Try another word, or browse the <a href="${wikiPrefix}troubleshooting/">Troubleshooting</a> page.</p>`;
        return;
      }

      resultsEl.innerHTML = matches
        .map(({ page, sectionHits }) => {
          const jumps = sectionHits.length
            ? `<div class="wiki-hit-sections">${sectionHits
                .map((s) => `<a href="${wikiPrefix}${page.slug}/#${s.id}">${esc(s.title)}</a>`)
                .join("")}</div>`
            : "";
          return `<div class="wiki-hit">
            <a class="wiki-hit-page" href="${wikiPrefix}${page.slug}/">
              <span class="wiki-hit-group">${esc(page.group)}</span>
              <b>${esc(page.title)}</b>
              <p>${esc(page.summary)}</p>
            </a>
            ${jumps}
          </div>`;
        })
        .join("");
    }

    input.addEventListener("input", () => render(input.value));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && input.value) {
        input.value = "";
        render("");
      }
    });
    if (clearBtn) {
      clearBtn.addEventListener("click", () => {
        input.value = "";
        render("");
        input.focus();
      });
    }
  }

  // ---- On-this-page scroll spy (re-initialized cleanly on every guide swap)
  let teardownScrollSpy = null;
  let markActiveSection = null;

  function initScrollSpy() {
    if (teardownScrollSpy) {
      teardownScrollSpy();
      teardownScrollSpy = null;
      markActiveSection = null;
    }

    const onpageLinks = [...document.querySelectorAll(".wiki-onpage nav a")];
    const headings = [...document.querySelectorAll(".wiki-text h2[id]")];
    if (!onpageLinks.length || !headings.length) return;

    const byId = new Map(onpageLinks.map((a) => [a.getAttribute("href").slice(1), a]));
    let activeId = "";
    const setActive = (id) => {
      if (!id || id === activeId) return;
      activeId = id;
      onpageLinks.forEach((a) => a.classList.toggle("is-active", a.getAttribute("href") === `#${id}`));
    };
    markActiveSection = (id) => {
      if (byId.has(id)) setActive(id);
    };

    const pickCurrent = () => {
      const offset = 132;
      let current = headings[0].id;
      for (const h of headings) {
        if (h.getBoundingClientRect().top <= offset) current = h.id;
        else break;
      }
      if (window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 24) {
        current = headings[headings.length - 1].id;
      }
      setActive(current);
    };

    let raf = 0;
    const onScroll = () => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        pickCurrent();
      });
    };
    const onHash = () => {
      const h = location.hash.slice(1);
      if (byId.has(h)) setActive(h);
    };

    addEventListener("scroll", onScroll, { passive: true });
    addEventListener("hashchange", onHash);
    pickCurrent();

    teardownScrollSpy = () => {
      if (raf) cancelAnimationFrame(raf);
      removeEventListener("scroll", onScroll);
      removeEventListener("hashchange", onHash);
    };
  }

  // Close the compact mobile "On this page" drawer after picking a section.
  document.addEventListener("click", (e) => {
    const a = e.target.closest(".wiki-jump a");
    if (!a) return;
    const details = a.closest("details");
    if (details) details.open = false;
  });

  // ---- Soft navigation (keep header, footer, and sidebar mounted across wiki pages)
  const pageCache = new Map(); // canonical pathname key -> Promise<PageSnapshot> | PageSnapshot
  let currentRoute = parseWikiRoute(location.href);
  let navSeq = 0;

  function extractSnapshot(doc, route) {
    const baseUrl = new URL(route.key, location.origin).href;
    const main = doc.querySelector("main.wiki");
    if (!main) return null;

    // Clone before mutating links so we never touch live DOM when seeding from `document`.
    const mainClone = main.cloneNode(true);
    pinRelativeUrls(mainClone, baseUrl);

    const shell = mainClone.querySelector(".wiki-shell");
    const wikiMain = mainClone.querySelector(".wiki-main");
    const onpageNav = mainClone.querySelector(".wiki-onpage nav");

    const getAttr = (sel, attr) => {
      const el = doc.head.querySelector(sel);
      return el ? el.getAttribute(attr) : null;
    };

    return {
      kind: route.kind,
      slug: (shell && shell.dataset.wikiSlug) || route.slug,
      pageTitle: (shell && shell.dataset.wikiTitle) || "",
      docTitle: doc.title,
      description: getAttr('meta[name="description"]', "content"),
      canonical: getAttr('link[rel="canonical"]', "href"),
      ogTitle: getAttr('meta[property="og:title"]', "content"),
      ogDesc: getAttr('meta[property="og:description"]', "content"),
      ogUrl: getAttr('meta[property="og:url"]', "content"),
      mainClass: mainClone.className,
      fullMainHtml: mainClone.innerHTML,
      articleMainHtml: wikiMain ? wikiMain.innerHTML : null,
      onpageNavHtml: onpageNav ? onpageNav.innerHTML : "",
    };
  }

  function loadPage(route) {
    const cached = pageCache.get(route.key);
    if (cached) return Promise.resolve(cached);

    const fetchUrl = new URL(route.key, location.origin).href;
    const pending = fetch(fetchUrl, { headers: { Accept: "text/html" } })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.text();
      })
      .then((markup) => {
        const doc = new DOMParser().parseFromString(markup, "text/html");
        const snap = extractSnapshot(doc, route);
        if (!snap) throw new Error("Missing wiki main container");
        pageCache.set(route.key, snap);
        return snap;
      })
      .catch((err) => {
        pageCache.delete(route.key);
        throw err;
      });

    pageCache.set(route.key, pending);
    return pending;
  }

  function updateHead(snap, route) {
    document.title = snap.docTitle;
    const setAttr = (sel, attr, val) => {
      if (val == null) return;
      const el = document.head.querySelector(sel);
      if (el) el.setAttribute(attr, val);
    };
    setAttr('meta[name="description"]', "content", snap.description);
    setAttr('link[rel="canonical"]', "href", snap.canonical);
    setAttr('meta[property="og:title"]', "content", snap.ogTitle);
    setAttr('meta[property="og:description"]', "content", snap.ogDesc);
    setAttr('meta[property="og:url"]', "content", snap.ogUrl);

    const enLang = document.querySelector('.nav-lang a[hreflang="en"]');
    if (enLang) enLang.setAttribute("href", route.key);
  }

  function updateSidebarState(slug, title) {
    for (const a of document.querySelectorAll(".wiki-sidebar a[data-wiki-slug]")) {
      if (a.dataset.wikiSlug === slug) a.setAttribute("aria-current", "page");
      else a.removeAttribute("aria-current");
    }
    const mobileNav = document.querySelector(".wiki-mobile-nav");
    if (mobileNav) {
      mobileNav.open = false;
      const label = mobileNav.querySelector("summary b");
      if (label && title) label.textContent = title;
    }
    const shell = document.querySelector(".wiki-shell");
    if (shell) {
      shell.dataset.wikiSlug = slug;
      if (title) shell.dataset.wikiTitle = title;
    }
  }

  function scrollToTarget(hash, savedScrollY) {
    if (hash) {
      const id = decodeURIComponent(hash.slice(1));
      const target = id ? document.getElementById(id) : null;
      if (target) {
        target.scrollIntoView({ behavior: "instant", block: "start" });
        if (markActiveSection) markActiveSection(id);
        return;
      }
    }
    window.scrollTo({ top: typeof savedScrollY === "number" ? savedScrollY : 0, behavior: "instant" });
    const nav = document.querySelector(".nav");
    if (nav) nav.classList.toggle("stuck", window.scrollY > 8);
  }

  // Screen-reader announcement region for client-side route changes.
  let liveRegion = null;
  function announce(text) {
    if (!liveRegion) {
      liveRegion = document.createElement("span");
      liveRegion.className = "visually-hidden";
      liveRegion.setAttribute("aria-live", "polite");
      document.body.append(liveRegion);
    }
    liveRegion.textContent = text;
  }

  function animateSwap(el) {
    if (!el) return;
    el.classList.remove("is-loading", "is-entering");
    void el.offsetWidth;
    el.classList.add("is-entering");
  }

  async function navigate(route, { push = true, savedScrollY = null } = {}) {
    const seq = ++navSeq;
    const liveMain = document.querySelector("main.wiki");
    if (!liveMain) {
      location.href = route.href;
      return;
    }

    // Save current scroll position in history state before pushing a new entry.
    if (push) {
      try {
        history.replaceState(
          { ...(history.state || {}), wiki: true, scrollY: window.scrollY },
          "",
          location.href
        );
      } catch {
        // Ignore restricted history states.
      }
    }

    const wikiMainEl = document.querySelector(".wiki-main");
    let loadingTimer = 0;
    const isCached = pageCache.has(route.key) && !(pageCache.get(route.key) instanceof Promise);
    if (!isCached && wikiMainEl) {
      loadingTimer = setTimeout(() => {
        if (seq === navSeq) wikiMainEl.classList.add("is-loading");
      }, 120);
    }

    let snap;
    try {
      snap = await loadPage(route);
    } catch {
      clearTimeout(loadingTimer);
      if (seq === navSeq) location.href = route.href;
      return;
    }
    clearTimeout(loadingTimer);
    if (seq !== navSeq) return;

    if (push) {
      history.pushState({ wiki: true, scrollY: 0 }, "", route.href);
    }

    const currentShell = document.querySelector(".wiki-shell");
    const currentWikiMain = document.querySelector(".wiki-main");
    const currentOnpage = document.querySelector(".wiki-onpage");

    if (route.kind === "guide" && currentShell && currentWikiMain && snap.articleMainHtml !== null) {
      // Guide -> Guide: keep header, footer, AND sidebar untouched; swap only center + outline.
      updateSidebarState(snap.slug, snap.pageTitle);
      currentWikiMain.innerHTML = snap.articleMainHtml;
      animateSwap(currentWikiMain);

      if (currentOnpage) {
        const nav = currentOnpage.querySelector("nav");
        if (nav) nav.innerHTML = snap.onpageNavHtml;
        currentOnpage.hidden = !snap.onpageNavHtml;
        currentOnpage.scrollTop = 0;
      }

      updateHead(snap, route);
      currentRoute = route;
      initScrollSpy();
      scrollToTarget(route.hash, savedScrollY);
      announce(snap.pageTitle || snap.docTitle);
      return;
    }

    // Index <-> Guide: keep header and footer untouched; swap only <main>.
    if (teardownScrollSpy) {
      teardownScrollSpy();
      teardownScrollSpy = null;
      markActiveSection = null;
    }
    liveMain.className = snap.mainClass;
    liveMain.innerHTML = snap.fullMainHtml;
    const nextWikiMain = liveMain.querySelector(".wiki-main");
    if (nextWikiMain) animateSwap(nextWikiMain);

    updateHead(snap, route);
    currentRoute = route;
    if (route.kind === "index") {
      initSearch();
    } else {
      initScrollSpy();
    }
    scrollToTarget(route.hash, savedScrollY);
    announce(snap.pageTitle || snap.docTitle);
  }

  function warmAllWikiPages() {
    if (!canSoftNav) return;
    if (navigator.connection && navigator.connection.saveData) return;
    const routes = [];
    const seen = new Set(currentRoute ? [currentRoute.key] : []);
    for (const a of document.querySelectorAll(".wiki-sidebar a[href], .wiki-cards a[href], .wiki-help-card a[href]")) {
      const r = parseWikiRoute(a.href);
      if (r && !seen.has(r.key)) {
        seen.add(r.key);
        routes.push(r);
      }
    }
    if (!routes.length) return;

    let idx = 0;
    const worker = () => {
      if (idx >= routes.length) return;
      const r = routes[idx++];
      loadPage(r).catch(() => {}).finally(worker);
    };
    const start = () => {
      worker();
      worker();
    };
    if ("requestIdleCallback" in window) requestIdleCallback(start, { timeout: 2500 });
    else setTimeout(start, 600);
  }

  // ---- Boot
  initSearch();
  initScrollSpy();

  if (canSoftNav && currentRoute) {
    // Pin relative paths in persistent chrome before any pushState changes URL depth.
    pinRelativeUrls(document.head, location.href);
    pinRelativeUrls(document.querySelector("header.nav"), location.href);
    pinRelativeUrls(document.querySelector("footer.credits"), location.href);
    pinRelativeUrls(document.querySelector(".wiki-sidebar"), location.href);

    const initialSnap = extractSnapshot(document, currentRoute);
    if (initialSnap) pageCache.set(currentRoute.key, initialSnap);

    if ("scrollRestoration" in history) history.scrollRestoration = "manual";
    try {
      history.replaceState(
        { ...(history.state || {}), wiki: true, scrollY: window.scrollY },
        "",
        location.href
      );
    } catch {
      // Ignore restricted history states.
    }

    // Prefetch on hover, keyboard focus, or touch start so navigation lands immediately.
    const maybePrefetch = (e) => {
      const a = e.target && e.target.closest ? e.target.closest("a[href]") : null;
      if (!a) return;
      const r = parseWikiRoute(a.href);
      if (r && !pageCache.has(r.key)) loadPage(r).catch(() => {});
    };
    document.addEventListener("pointerover", maybePrefetch, { passive: true });
    document.addEventListener("focusin", maybePrefetch, { passive: true });
    document.addEventListener("touchstart", maybePrefetch, { passive: true });

    // Intercept clicks on internal wiki links.
    document.addEventListener("click", (e) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) {
        return;
      }
      const a = e.target && e.target.closest ? e.target.closest("a[href]") : null;
      if (!a || a.hasAttribute("download")) return;
      if (a.target && a.target !== "_self") return;
      // Let language switcher links follow lang.js / normal behavior.
      if (a.closest(".nav-lang")) return;

      const route = parseWikiRoute(a.href);
      if (!route) return;

      // Same page: handle hash jump or scroll to top without re-rendering.
      if (currentRoute && route.key === currentRoute.key) {
        const mobileNav = document.querySelector(".wiki-mobile-nav");
        if (mobileNav) mobileNav.open = false;
        if (route.hash) {
          try {
            history.replaceState(
              { ...(history.state || {}), wiki: true, scrollY: window.scrollY },
              "",
              location.href
            );
          } catch {
            // Ignore restricted history states.
          }
          return; // Let native anchor jump run cleanly.
        }
        e.preventDefault();
        if (location.hash) {
          history.pushState({ wiki: true, scrollY: 0 }, "", route.key);
        }
        window.scrollTo({ top: 0, behavior: "smooth" });
        return;
      }

      e.preventDefault();
      navigate(route, { push: true });
    });

    // Handle browser Back / Forward between wiki pages seamlessly.
    addEventListener("popstate", (e) => {
      const route = parseWikiRoute(location.href);
      if (!route) return;
      if (currentRoute && route.key === currentRoute.key) {
        const savedY = e.state && typeof e.state.scrollY === "number" ? e.state.scrollY : null;
        scrollToTarget(route.hash, savedY);
        return;
      }
      const savedY = e.state && typeof e.state.scrollY === "number" ? e.state.scrollY : 0;
      navigate(route, { push: false, savedScrollY: savedY });
    });

    warmAllWikiPages();
  }
})();
