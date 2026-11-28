// The redesign mockup's own behaviour, beside app.js: the bar that turns to glass, the hero's
// beams following the pointer, and the tour's tabs projecting their screen.
(() => {
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // The bar turns to glass once the page has scrolled under it.
  const nav = document.querySelector(".nav");
  const stick = () => nav.classList.toggle("stuck", scrollY > 8);
  stick();
  addEventListener("scroll", stick, { passive: true });

  // The beams lean toward the pointer, the near ones more than the far ones.
  const hero = document.querySelector(".bhero");
  const lanes = document.getElementById("lanes");
  if (hero && lanes && !reduced && matchMedia("(pointer: fine)").matches) {
    let frame = 0;
    hero.addEventListener("pointermove", (e) => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const r = hero.getBoundingClientRect();
        lanes.style.setProperty("--px", `${(((e.clientX - r.left) / r.width - 0.5) * 60).toFixed(1)}px`);
        lanes.style.setProperty("--py", `${(((e.clientY - r.top) / r.height - 0.5) * 44).toFixed(1)}px`);
      });
    });
    hero.addEventListener("pointerleave", () => { lanes.style.setProperty("--px", "0px"); lanes.style.setProperty("--py", "0px"); });
  }

  // The tour: a tab shows its screen, and the cone of light runs from that tab to the window.
  const stage = document.querySelector(".tour-stage");
  const img = document.getElementById("tour-shot");
  if (!stage || !img) return;
  const tabs = [...stage.querySelectorAll(".tour-tabs button")];
  const win = stage.querySelector(".tour-window");
  const cone = document.getElementById("cone");
  const railIcons = [...stage.querySelectorAll(".tw-rail i[data-for]")];
  const alts = {
    home: "Anibeam's Home: the show to pick up and the shows to continue",
    library: "The Library grid with every show as a poster",
    series: "A show's page with its banner, details and episodes",
  };
  const wide = matchMedia("(min-width: 900px)").matches;
  const src = (k) => `${AB.ROOT}img/tour/${k}${wide ? "" : "-1134"}.webp`;
  const warm = () => Object.keys(alts).forEach((k) => { new Image().src = src(k); });
  if (wide) { if ("requestIdleCallback" in window) requestIdleCallback(warm, { timeout: 3000 }); else setTimeout(warm, 1500); }

  function aim() {
    const active = tabs.find((t) => t.getAttribute("aria-selected") === "true");
    if (!active || !cone) return;
    const box = cone.getBoundingClientRect();
    const t = active.getBoundingClientRect();
    const w = win.getBoundingClientRect();
    const x0 = t.right + 14 - box.left;
    const y0 = t.top + t.height / 2 - box.top;
    const x1 = w.left + 30 - box.left;
    cone.style.clipPath = `polygon(${x0}px ${y0 - 3}px, ${x1}px ${w.top - box.top}px, ${x1}px ${w.bottom - box.top}px, ${x0}px ${y0 + 3}px)`;
  }
  let pending = null;
  function show(key, focus) {
    tabs.forEach((b) => {
      const on = b.dataset.screen === key;
      b.setAttribute("aria-selected", String(on));
      b.tabIndex = on ? 0 : -1;
      if (on && focus) b.focus();
    });
    railIcons.forEach((i) => i.classList.toggle("on", i.dataset.for === (key === "series" ? "library" : key)));
    aim();
    img.classList.add("swap");
    // A screen that loads after a later tab was picked is dropped, so the last pick always wins.
    const next = new Image();
    pending = next;
    next.onload = () => {
      if (pending !== next) return;
      img.removeAttribute("srcset"); img.src = next.src; img.alt = AB.t(alts[key]); img.classList.remove("swap");
    };
    next.onerror = () => { if (pending === next) img.classList.remove("swap"); };
    next.src = src(key);
  }
  tabs.forEach((b, i) => {
    b.tabIndex = i === 0 ? 0 : -1;
    b.addEventListener("click", () => show(b.dataset.screen));
    b.addEventListener("keydown", (e) => {
      const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
      if (!step) return;
      e.preventDefault();
      show(tabs[(i + step + tabs.length) % tabs.length].dataset.screen, true);
    });
  });
  railIcons.forEach((i) => i.classList.toggle("on", i.dataset.for === "home"));
  aim();
  addEventListener("resize", aim);
  if ("ResizeObserver" in window) new ResizeObserver(aim).observe(stage);
  document.fonts && document.fonts.ready.then(aim);
})();

// Art marks: a poster is AI-made until img/posters/credits.json names the person who drew it.
(() => {
  const posters = [...document.querySelectorAll(".lanes .poster img, .sp img, .bl-stop img, .pickup-poster")];
  const slug = (img) => (img.getAttribute("src").match(/posters\/([a-z0-9-]+)\.webp/) || [])[1];
  const marks = posters.filter(slug).map((img) => {
    let box = img.parentElement;
    if (!box.matches(".poster")) {
      box = document.createElement("span");
      box.className = img.closest(".bl-stop") ? "bl-art" : "sp-art";
      if (img.classList.contains("pickup-poster")) { box.className = "pk-art"; box.style.position = "relative"; box.style.alignSelf = "start"; }
      img.before(box);
      box.append(img);
    }
    const m = document.createElement("i");
    m.className = "art-mark";
    m.textContent = "AI";
    box.append(m);
    return [slug(img), m];
  });
  // The tour's screenshots are of a library of the same AI posters, so the window carries one mark too.
  const screen = document.querySelector(".tw-screen");
  if (screen) {
    const m = document.createElement("i");
    m.className = "art-mark tour-mark";
    m.textContent = "AI";
    screen.append(m);
  }
  fetch(AB.ROOT + "img/posters/credits.json").then((r) => (r.ok ? r.json() : null)).then((c) => {
    const by = (c && c.artists) || {};
    for (const [s, m] of marks) if (by[s] && by[s].by) { m.textContent = by[s].by; m.classList.add("by"); }
  }).catch(() => {});

  // The note about the art opens above its button: on hover with a mouse, and a click keeps it open.
  const btn = document.querySelector(".art-note-btn");
  const note = document.getElementById("art-note");
  if (btn && note) {
    let pinned = false, timer = 0;
    const show = (on) => { clearTimeout(timer); note.hidden = !on; btn.setAttribute("aria-expanded", String(on)); if (!on) pinned = false; };
    btn.addEventListener("click", () => { pinned = !(pinned && !note.hidden); show(pinned); });
    if (matchMedia("(hover: hover)").matches) {
      const leave = () => { timer = setTimeout(() => { if (!pinned) show(false); }, 250); };
      for (const el of [btn, note]) {
        el.addEventListener("pointerenter", () => show(true));
        el.addEventListener("pointerleave", leave);
      }
    }
    document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !note.hidden) { show(false); btn.focus(); } });
    document.addEventListener("click", (e) => { if (!note.hidden && !e.target.closest(".art-note-wrap")) show(false); });
  }
})();
