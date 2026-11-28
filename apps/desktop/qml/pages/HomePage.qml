// Home: the page the app opens on, ordered by what the app is for. First the hero: the
// series to go on with, one at a time and the same for every show (#1468): the backdrop across
// the window, the poster in front, the logo or the title, the episode, two lines of the
// show's synopsis and the two actions. It steps on by itself after a stretch with no input,
// or by hand with its arrows. Under it Continue, one row of episode cards that steps along
// with the hero, the hero's series at its left and lit; with more series than fit, the row
// runs on in a loop and its right edge falls into shadow. A series on hold is in neither; it
// waits on a shelf of its own further down.
// Then the week: seven day columns from today with the airing series' posters placed on the
// day their next episode lands, and a Later column for the rest. Then the series with episodes
// waiting, a row not started yet, and the library's genres as a way in. A series dropped on
// the tracker is in none of those: it closes the page on a shelf of its own, a reminder of
// what can leave the disk. Every shelf is a plain grid whose column count comes from the
// width, and a shelf that could run long stops at a row or two with "See all".
//
// With no library yet Home holds the same empty state as Library and nothing else (#1006):
// rows of the season's shows had nothing to do with the person who opened it.
import QtQuick
import QtQuick.Shapes
import QtQuick.Window
import com.marcusrosado.Anibeam

Item {
    id: page
    objectName: "home-page"
    property var props: ({})
    property string title: words.t("home-title")
    property alias scrollY: scroll.contentY
    function escapePressed() { return false }
    // The arrows with nothing focused go to the scroll's grid keys (Down lands on the hero's
    // button or the first card)
    Keys.onPressed: function(event) { hero.lastInput = Date.now(); scroll.keyPressed(event) }
    // The pointer moving anywhere on the page holds the hero's turn too
    HoverHandler { onPointChanged: hero.lastInput = Date.now() }

    property real nowMs: Date.now()
    property var shelf: []        // the library, last viewed first
    property var airingRows: []   // releasing, on disk, not done with
    property bool libraryEmpty: false
    // A read that failed shows Try again on the page, not a blank Home
    property bool loadFailed: false
    function ago(at) {
        var secs = Math.max(0, nowMs / 1000 - at)
        if (secs < 3600) return words.t("home-ago-minutes", { count: Math.max(1, Math.floor(secs / 60)) })
        if (secs < 86400) return words.t("home-ago-hours", { count: Math.floor(secs / 3600) })
        return words.t("home-ago-days", { count: Math.floor(secs / 86400) })
    }
    Timer { interval: 30000; running: true; repeat: true; onTriggered: page.nowMs = Date.now() }

    readonly property real gap: theme.space(4)

    function viewedAt(c) { return c.last_viewed_at ? c.last_viewed_at : 0 }
    function watchedOf(c) { return c.watched !== null && c.watched !== undefined ? c.watched : 0 }
    function droppedOf(c) { return c.list_status === "Dropped" }
    function pausedOf(c) { return c.list_status === "Paused" }

    // Continue: the series to go on with. An episode stopped short of its end comes first,
    // else the series' next episode on disk; a series with neither is caught up and is
    // passed over for the next one.
    // A series counts as started when it was played here or when the tracker says it is
    // being watched with episodes seen, since plenty get watched somewhere else. They are
    // ordered by the later of the last play here and, while an episode is out and unseen,
    // the day that episode arrived: a new episode of a show being followed is the obvious
    // next watch. The hero cycles through all of them and Continue holds them all. At
    // most sixteen series are read; a dropped one is never among them, and one on hold is
    // left out too: it is on hold for a reason (#1464).
    function startedOf(c) {
        if (page.droppedOf(c) || c.list_status === "Completed") return false
        if (page.pausedOf(c)) return false
        if (page.viewedAt(c) > 0) return true
        return page.watchedOf(c) > 0 && (c.list_status === "Watching" || c.list_status === "Repeating")
    }
    function continueAt(c) {
        var fresh = c.watched_state === "Behind" && c.latest_activity_at ? c.latest_activity_at : 0
        return Math.max(page.viewedAt(c), fresh)
    }
    function pickContinue() {
        var out = []
        var started = shelf.filter(page.startedOf).sort(function(a, b) { return page.continueAt(b) - page.continueAt(a) })
        for (var i = 0; i < started.length && i < 16; i++) {
            var c = started[i]
            var r = Door.getSeries(c.id)
            if (r.error) continue
            var d = r.reply.detail
            var eps = d.episodes || []
            var partial = null, next = null
            for (var k = 0; k < eps.length; k++) {
                var e = eps[k]
                // Stopped before the outro (85 percent when no outro is known): a stop in
                // the credits is a finished episode, a stop before them is not
                // An episode a torrent is still writing is never what to go on with
                if (e.downloading !== undefined && e.downloading !== null) continue
                if (!e.watched && e.stopped_short && e.resume && e.resume.duration > 0)
                    partial = e
                if (d.next_up && e.file === d.next_up) next = e
            }
            var ep = partial || next
            if (ep) out.push({ card: c, detail: d, episode: ep, partial: partial !== null })
        }
        return out
    }
    property var continueList: []   // [{ card, detail, episode, partial }]
    // The hero's turn: every series in Continue, one at a time, in Continue's order, so the
    // hero's place is the Continue card at the row's left. Its slides hang off a model of
    // series ids kept in place across reloads (syncHero), so a read that leaves a series in the
    // hero keeps its slide, a crossfade under way and the focus inside it; each slide reads its
    // entry from heroEntries by id, which a reload replaces.
    readonly property var heroList: continueList
    readonly property var heroEntries: {
        var m = {}
        for (var i = 0; i < heroList.length; i++) m[heroList[i].card.id] = heroList[i]
        return m
    }
    ListModel { id: heroIds }
    function syncHero() {
        var want = heroList.map(function(e) { return e.card.id })
        for (var i = heroIds.count - 1; i >= 0; i--)
            if (want.indexOf(heroIds.get(i).sid) < 0) heroIds.remove(i)
        for (var k = 0; k < want.length; k++) {
            var at = -1
            for (var j = k; j < heroIds.count; j++) if (heroIds.get(j).sid === want[k]) { at = j; break }
            if (at < 0) heroIds.insert(k, { sid: want[k] })
            else if (at !== k) heroIds.move(at, k, 1)
        }
        hero.afterSync()
    }
    onHeroListChanged: syncHero()
    readonly property var continueIds: {
        var m = {}
        for (var i = 0; i < continueList.length; i++) m[continueList[i].card.id] = true
        return m
    }

    function reload() {
        var r = Door.listLibrary("All", "", "LastViewed", "Desc", false)
        loadFailed = !!r.error && shelf.length === 0
        // A failed read is not an empty library, whatever the last read said
        if (r.error) { if (loadFailed) libraryEmpty = false; frame.fail(r.error, page.reload); return }
        shelf = r.reply.series
        airingRows = r.reply.airing || []
        var n = r.reply.counts || {}
        // Hidden series are still a library, as Library counts them
        libraryEmpty = (n.all || 0) === 0 && (n.watched || 0) === 0 && (n.hidden || 0) === 0
        continueList = pickContinue()
        nowMs = Date.now()
    }
    // ---- Recommended for you (#779): the recommender's ranking held to one scope, best first.
    // The scope is a Home preference; the core refits after every tracker refresh and says so
    // with RecommendedRefreshed.
    readonly property var recScopes: ["All", "InLibrary", "OnList"]
    readonly property string recScope: Door.preferences.home_recommended || "All"
    property var recommended: null
    readonly property var recList: recommended ? recommended.shows : []
    // Read off the Qt thread: the read parses the whole ranking
    property int recToken: -1
    function loadRecommended() { page.recToken = Door.callAsync("ListRecommended", { scope: page.recScope }) }
    Timer { id: recDebounce; interval: 500; onTriggered: page.loadRecommended() }
    function setRecScope(scope) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.home_recommended = scope
        var r = Door.setPreferences(p)
        if (r && r.error) frame.fail(r.error)
    }
    onRecScopeChanged: loadRecommended()
    // A show the library has opens its series; any other opens its own page
    function openShow(s) {
        if (s.owned) frame.go("series", { id: s.owned }, s.title)
        else frame.go("show", { anilist: s.anilist_id }, s.title)
    }
    Timer { id: debounce; interval: 500; onTriggered: page.reload() }
    Connections {
        target: Door
        function onSeriesChanged() { if (!debounce.running) debounce.start(); recDebounce.restart() }
        function onSeriesRemoved() { if (!debounce.running) debounce.start(); recDebounce.restart() }
        function onPreferencesChanged(p) { if (!debounce.running) debounce.start() }
        function onEvent(envelope) { if (envelope.kind === "RecommendedRefreshed") page.loadRecommended() }
        function onTrackersChanged() { recDebounce.restart() }
        function onCallDone(token, reply) {
            if (token !== page.recToken) return
            page.recToken = -1
            if (!reply.error) page.recommended = reply.reply.recommended
        }
    }
    Component.onCompleted: {
        reload()
        loadRecommended()
        // AniList only when the last fit is stale; the fit ends in RecommendedRefreshed
        Door.call("RefreshRecommended", { force: false })
    }

    function playFile(file, title) { if (file) frame.go("player", { file: file }, title) }
    function wholeFirst(d) { return d && d.episodes && d.episodes.length && (d.episodes[0].downloading === undefined || d.episodes[0].downloading === null) ? d.episodes[0].file : null }
    function openSeries(c) { frame.go("series", { id: c.id }, c.title) }
    function playSeries(c) {
        var r = Door.getSeries(c.id)
        var d = !r.error ? r.reply.detail : null
        var file = d ? (d.next_up ? d.next_up : (c.kind === "Movie" ? page.wholeFirst(d) : null)) : null
        if (file) playFile(file, c.title)
        else openSeries(c)
    }

    // ---- The lists. A series in Continue shows in the hero and Continue, nowhere else.
    readonly property var airingIds: {
        var m = {}
        for (var i = 0; i < airingRows.length; i++) m[airingRows[i].id] = true
        return m
    }
    // The week (#236): nine day rows, two days back, today and six ahead, every one shown
    // even when nothing is on it. Each row holds the episodes the schedule puts on that day,
    // the ones that already aired included, so this morning's episode stays on Today and
    // says whether it landed. A predicted next episode is placed by its guess; a series with
    // nothing in the nine days and no date at all is "No date yet".
    // The calendar day the week is built for: it changes once a day, not on every tick of
    // nowMs, so the rows and their entries are rebuilt only when the day turns. The day turning
    // reloads too, so each card's window of the schedule moves with it.
    readonly property string dayKey: Qt.formatDate(new Date(nowMs), "yyyy-MM-dd")
    onDayKeyChanged: if (shelf.length) reload()
    readonly property var week: {
        var now = new Date(dayKey + "T00:00:00")
        var start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000
        var days = []
        for (var i = -2; i < 7; i++) {
            var d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + i)
            days.push({ label: i === 0 ? words.t("home-today") : i === 1 ? words.t("home-tomorrow") : i === -1 ? words.t("home-yesterday") : Qt.formatDate(d, words.t("home-weekday-format")),
                        date: Qt.formatDate(d, words.t("home-date-format")), today: i === 0, past: i < 0,
                        from: d.getTime() / 1000, cards: [] })
        }
        var end = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 7).getTime() / 1000
        function dayOf(at) {
            for (var k = days.length - 1; k >= 0; k--) if (at >= days[k].from) return at < end ? k : -1
            return -1
        }
        var later = []
        for (var r = 0; r < airingRows.length; r++) {
            var c = airingRows[r]
            var drops = (c.week_airings || []).slice()
            // The next episode the schedule window does not hold: a predicted one, or a TVmaze
            // show's next season, which is not the card's season
            var n = c.next_airing
            if (n && !drops.some(function(a) { return a.episode === n.episode && (a.season || 1) === (n.season || 1) }))
                drops.push({ episode: n.episode, through: n.through, season: n.season, at: n.at, predicted: n.predicted, here: false })
            var placed = false
            for (var j = 0; j < drops.length; j++) {
                var k = dayOf(drops[j].at)
                if (k < 0) continue
                days[k].cards.push({ card: c, drop: drops[j] })
                placed = true
            }
            if (!placed) later.push({ card: c, drop: null })
        }
        for (var q = 0; q < days.length; q++) days[q].cards.sort(function(a, b) { return a.drop.at - b.drop.at })
        if (later.length) days.push({ label: words.t("home-no-date-yet"), date: "", today: false, past: false, cards: later, later: true })
        return days
    }
    // Where an episode that is due stands: "future" until its time, then "here" with its file
    // of that season on disk or watched already (the core says), "downloading" while a torrent
    // writes it, and "late" when it is out and nothing has arrived, or a predicted one that is
    // not out yet
    function dropState(c, drop, nowMs) {
        if (!drop) return "none"
        if (drop.at * 1000 > nowMs) return "future"
        if (drop.here) return "here"
        var last = drop.through || drop.episode
        if ((c.downloading || []).some(function(d) { return d.number >= drop.episode && d.number <= last })) return "downloading"
        return drop.predicted ? "overdue" : "late"
    }
    // This week split by the two questions (#326): what is out and waiting (here and not
    // watched, downloading, or aired and not downloaded), the next episode to air, and the
    // rest still to come in air order. An episode here and watched has nothing left to say.
    // Built from the episodes' standings, not from the clock: the clock ticks every 30 s, and
    // a fresh split would rebuild every card under it, dropping focus and hover. `weekKey`
    // reads the clock and changes only when a standing does; the split follows the key.
    function weekEntries() {
        var out = [], ahead = [], later = [], aired = [], seen = {}
        for (var i = 0; i < week.length; i++) {
            var day = week[i]
            for (var k = 0; k < day.cards.length; k++) {
                var e = day.cards[k]
                if (day.later) { later.push({ card: e.card, drop: null, state: "none", day: i }); continue }
                var st = page.dropState(e.card, e.drop, nowMs)
                // Yesterday's and today's episodes stay on their day once they air, saying how
                // long ago, watched or not, so the board still answers whether something came out
                if ((i === 1 || i === 2) && (st === "here" || st === "downloading" || st === "late"))
                    aired.push({ card: e.card, drop: e.drop, day: i, state: st, aired: true,
                                 watched: st === "here" && page.dropWatched(e.card, e.drop) })
                if (st === "here" || st === "downloading" || st === "late") {
                    if (st === "here" && page.dropWatched(e.card, e.drop)) continue
                    out.push({ card: e.card, drop: e.drop, day: i, state: st })
                } else ahead.push({ card: e.card, drop: e.drop, day: i, state: st })
            }
        }
        // One card a series: its latest episode out stands for the rest
        out.sort(function(a, b) { return b.drop.at - a.drop.at })
        out = out.filter(function(o) { if (seen[o.card.id]) return false; seen[o.card.id] = true; return true })
        out.sort(function(a, b) { return a.drop.at - b.drop.at })
        ahead.sort(function(a, b) { return a.drop.at - b.drop.at })
        aired.sort(function(a, b) { return a.drop.at - b.drop.at })
        return { out: out, ahead: ahead, later: later, aired: aired }
    }
    // Watched already: the tracker's progress reaches it, or with no tracker the core reads
    // the series caught up
    function dropWatched(c, drop) {
        if (c.watched !== null && c.watched !== undefined) return c.watched >= (drop.through || drop.episode)
        return c.watched_state === "CaughtUp"
    }
    // The key is each entry whole, its card, its episode and its standing, so anything a tile
    // shows changing rebuilds the week: a standing, and also a title when the title language
    // switches, a score, a hold or a new file on disk (#355)
    readonly property string weekKey: {
        var e = weekEntries()
        function entries(list) { return list.map(function(x) { return [x.card, x.drop, x.state] }) }
        return JSON.stringify([dayKey, entries(e.out), entries(e.ahead), entries(e.later), e.aired.map(function(x) { return [x.card.id, x.drop, x.state, x.watched] })])
    }
    property var weekSplit: ({ out: [], ahead: [], next: null, later: [], aired: [] })
    function splitWeek() {
        var e = weekEntries(), next = null
        for (var j = 0; j < e.ahead.length; j++) if (e.ahead[j].state === "future") { next = e.ahead.splice(j, 1)[0]; break }
        weekSplit = { out: e.out, ahead: e.ahead, next: next, later: e.later, aired: e.aired }
    }
    onWeekKeyChanged: splitWeek()
    // The board (#352): today and the six days after as columns, each with its episodes
    // still to come in air order, the next one first in its day. Today opens with the
    // episodes that already aired today (#715), over a Now line, and a Yesterday column
    // leads when something aired yesterday. A day with nothing on it
    // stays as a narrow column, so the week keeps its shape and a gap reads as a gap. A
    // guessed episode past its time and not out yet sits on today, after the next one; the
    // series with no date close the board in a column of their own.
    readonly property var board: {
        // Built from the day and the split alone, not from `week`, so a reload that leaves
        // every standing as it was keeps the board's tiles, their focus and hover
        var cols = [], split = weekSplit, now = new Date(dayKey + "T00:00:00")
        var overdue = split.ahead.filter(function(a) { return a.state === "overdue" })
        // Yesterday leads the board only when something aired on it
        var yesterday = split.aired.filter(function(a) { return a.day === 1 })
        if (yesterday.length) {
            var yd = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
            cols.push({ name: words.t("home-yesterday"), short: words.t("home-yesterday"), date: Qt.formatDate(yd, words.t("home-day-date-format")), shortDate: Qt.formatDate(yd, words.t("home-date-format")),
                        today: false, past: true, later: false, items: yesterday, next: null })
        }
        for (var o = 0; o < 7; o++) {
            var i = o + 2   // the week's index: it starts two days back
            var items = split.ahead.filter(function(a) { return a.day === i && a.state !== "overdue" })
            if (o === 0) items = split.aired.filter(function(a) { return a.day === i }).concat(items, overdue)
            // A copy marked as the next, since a model hands the tiles copies, never the same object
            var next = split.next && split.next.day === i ? { card: split.next.card, drop: split.next.drop, day: i, state: split.next.state, next: true } : null
            if (next) items.unshift(next)
            var d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + o)
            var name = o === 0 ? words.t("home-today") : o === 1 ? words.t("home-tomorrow") : Qt.formatDate(d, words.t("home-weekday-format"))
            cols.push({ name: name, short: o === 0 ? name : Qt.formatDate(d, words.t("home-weekday-short-format")),
                        date: o < 2 ? Qt.formatDate(d, words.t("home-day-date-format")) : Qt.formatDate(d, words.t("home-date-format")), shortDate: Qt.formatDate(d, words.t("home-date-format")),
                        today: o === 0, later: false, items: items, next: next })
        }
        if (split.later.length) cols.push({ name: words.t("home-no-date-yet"), short: words.t("home-no-date-yet"), date: "", shortDate: "", today: false, later: true, items: split.later, next: null })
        return cols
    }
    // Whether the board leaves out the days with nothing on them (#688), kept in the
    // preferences so it holds across launches
    readonly property bool hideEmptyDays: Door.preferences.home_hide_empty_days === true
    function setHideEmptyDays(on) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.home_hide_empty_days = on
        Door.setPreferences(p)
    }
    // "1 day 20 h 36 m" as parts, for the countdown's large figures
    function countdownParts(secs) {
        var t = Math.max(0, Math.floor(secs)), d = Math.floor(t / 86400), h = Math.floor(t % 86400 / 3600), m = Math.floor(t % 3600 / 60)
        var parts = []
        if (d > 0) parts.push({ n: d, unit: words.t("home-unit-days", { count: d }) })
        if (d > 0 || h > 0) parts.push({ n: h, unit: words.t("home-unit-hours") })
        parts.push({ n: Math.max(d > 0 || h > 0 ? 0 : 1, m), unit: words.t("home-unit-minutes") })
        return parts
    }
    readonly property int lateCount: {
        var n = 0
        for (var i = 0; i < week.length; i++)
            for (var k = 0; k < week[i].cards.length; k++) {
                var s = page.dropState(week[i].cards[k].card, week[i].cards[k].drop, nowMs)
                if (s === "late" || s === "overdue") n++
            }
        return n
    }
    readonly property bool weekFactCounts: lateCount === 0 && weekSplit.out.every(function(o) { return o.state !== "here" })
    readonly property string weekFact: {
        if (lateCount > 0) return words.t("home-week-late", { count: lateCount })
        var waiting = weekSplit.out.filter(function(o) { return o.state === "here" }).length
        if (waiting > 0) return words.t("home-week-waiting", { count: waiting })
        // The board names the next episode itself; the fact counts what is on it instead
        var coming = weekSplit.ahead.length + (weekSplit.next ? 1 : 0)
        if (coming > 0) return words.t("home-week-coming", { count: coming })
        var now = nowMs / 1000, next = null
        for (var i = 0; i < airingRows.length; i++) {
            var n = airingRows[i].next_airing
            if (n && n.at > now && (!next || n.at < next.n.at)) next = { n: n, c: airingRows[i] }
        }
        return next ? words.t("home-week-next", { title: next.c.title, time: shortCountdown(next.n.at - now) }) : ""
    }
    // Episodes waiting: behind on the tracker, or watched recently and not caught up
    readonly property var catchUpList: {
        var month = 45 * 86400, now = nowMs / 1000
        return shelf.filter(function(c) {
            if (page.continueIds[c.id] || page.airingIds[c.id] || page.droppedOf(c) || page.pausedOf(c)) return false
            if (c.watched_state === "Behind") return true
            return page.viewedAt(c) > now - month && c.watched_state !== "CaughtUp"
        })
    }
    // The series a file arrived in over the last fortnight, newest first; one Continue holds
    // already is left out, since its new episode is the card there, and one on hold has its
    // own shelf (#1464)
    readonly property var downloadedList: {
        var since = nowMs / 1000 - 14 * 86400
        return shelf.filter(function(c) { return c.downloaded_at && c.downloaded_at >= since && !page.continueIds[c.id] && !page.pausedOf(c) })
            .sort(function(a, b) { return b.downloaded_at - a.downloaded_at })
    }
    // Never started, best rated first
    readonly property var freshList: {
        var list = shelf.filter(function(c) {
            return !page.continueIds[c.id] && !page.airingIds[c.id] && !page.droppedOf(c) && !page.pausedOf(c) && page.viewedAt(c) === 0
                && page.watchedOf(c) === 0 && (c.episodes_on_disk || 0) > 0
        })
        return list.sort(function(a, b) { return (b.community_score || 0) - (a.community_score || 0) })
    }
    // On hold, on the tracker or on this device: the ones with an episode on disk to play
    // first, then the one touched here or given a new episode most lately
    function lastTouchOf(c) { return Math.max(page.viewedAt(c), c.latest_activity_at || 0) }
    readonly property var holdList: {
        return shelf.filter(page.pausedOf).sort(function(a, b) {
            var pa = page.toWatchOf(a) > 0 ? 1 : 0, pb = page.toWatchOf(b) > 0 ? 1 : 0
            return pb - pa || page.lastTouchOf(b) - page.lastTouchOf(a)
        })
    }
    // Dropped on the tracker and still on disk
    readonly property var droppedList: shelf.filter(page.droppedOf)
    readonly property int droppedFiles: droppedList.reduce(function(n, c) { return n + (c.episodes_on_disk || 0) }, 0)
    // Every genre in the library with its count, the biggest first
    readonly property var genreList: {
        var counts = {}
        for (var i = 0; i < shelf.length; i++) {
            var g = shelf[i].genres || []
            for (var k = 0; k < g.length; k++) counts[g[k]] = (counts[g[k]] || 0) + 1
        }
        var out = Object.keys(counts).map(function(name) { return { name: name, count: counts[name] } })
        return out.sort(function(a, b) { return b.count - a.count || (a.name < b.name ? -1 : 1) })
    }

    // "4d 18h", "3h 12m", "9m"
    function shortCountdown(secs) {
        var t = Math.max(0, Math.floor(secs))
        var d = Math.floor(t / 86400), h = Math.floor(t % 86400 / 3600), m = Math.floor(t % 3600 / 60)
        return d > 0 ? words.t("home-countdown-days", { days: d, hours: h }) : h > 0 ? words.t("home-countdown-hours", { hours: h, minutes: m })
            : words.t("home-countdown-minutes", { minutes: Math.max(1, m) })
    }

    // ---- A section heading: the name in the display cut, its one fact beside it, and an
    // action at the right
    component Heading: Item {
        id: heading
        property string text: ""
        property string fact: ""
        property alias action: actionSlot.data
        width: parent ? parent.width : 0
        height: theme.space(12)
        Row {
            id: headingRow
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            anchors.bottomMargin: theme.space(2)
            spacing: theme.space(3)
            Label { id: headingText; text: heading.text; role: "display"; font.pointSize: theme.typeHeading; font.variableAxes: ({ "wdth": 70, "wght": 420 }) }
            // Whole or not at all: a heading too narrow for its fact (On hold's split row)
            // drops it rather than running it into the action or the next heading
            Label {
                anchors.baseline: headingText.baseline
                text: heading.fact
                role: "ui"
                color: theme.textDim
                visible: implicitWidth <= heading.width - headingText.implicitWidth - headingRow.spacing
                                          - (actionSlot.visibleChildren.length > 0 ? actionSlot.width + theme.space(3) : 0)
            }
        }
        Item { id: actionSlot; anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.bottomMargin: theme.space(2); width: childrenRect.width; height: childrenRect.height }
    }

    // The newest episode number on disk, off the card's code (S02E03, EP 3)
    function newestOnDisk(c) {
        var m = /(\d+(?:\.\d+)?)$/.exec(c.code || "")
        return m ? Number(m[1]) : 0
    }
    // The episodes on disk past the tracker's progress; -1 with no tracker entry
    function toWatchOf(c) {
        if (c.watched === null || c.watched === undefined) return -1
        var n = c.episodes_on_disk || 0, top = newestOnDisk(c)
        return Math.min(n, Math.max(0, top - c.watched))
    }

    // What an Out now poster says under its title: when it aired, or why it cannot play yet
    function outLine(o) {
        var t = Qt.formatTime(new Date(o.drop.at * 1000), words.t("home-time-format")), day = week[o.day] ? week[o.day].label : ""
        if (o.state === "late") return words.t("home-out-not-downloaded")
        if (o.state === "downloading") return words.t("home-out-downloading")
        return words.t("home-out-when", { day: day, time: t })
    }
    // A sub-heading inside the week: a word and a dim count
    component WeekLabel: Row {
        property alias text: wl.text
        property alias fact: wf.text
        spacing: theme.space(2)
        Label { id: wl; role: "title"; font.pointSize: theme.typeNormal }
        Label { id: wf; anchors.baseline: wl.baseline; role: "ui"; color: theme.textDim; visible: text !== "" }
    }
    // The next episode to air, first in its day on the board: a band in its own tone with
    // the poster fading in from the right, the time left in large figures, then the title
    // and when. The one loud thing on the board. Opens the series.
    component NextTile: Corner {
        id: nextTile
        objectName: "home-week-next"
        property var entry: null
        readonly property bool gridStop: true
        readonly property var card: entry ? entry.card : ({})
        readonly property var drop: entry ? entry.drop : null
        readonly property var parts: drop ? page.countdownParts(drop.at - page.nowMs / 1000) : []
        readonly property color base: theme.bandBase(nextTone.tone, 0.5)
        readonly property color inkAccent: theme.bandAccent(nextTone.tone, base)
        readonly property color ink: "#f4f6f8"
        readonly property color inkDim: Qt.rgba(1, 1, 1, 0.68)
        radius: theme.radiusTile
        smoothing: theme.cornerSmoothing
        color: base
        borderColor: activeFocus ? theme.focusRing : nm.containsMouse ? Qt.rgba(1, 1, 1, 0.3) : "transparent"
        borderWidth: 1
        activeFocusOnTab: true
        Keys.onReturnPressed: page.openSeries(card)
        Keys.onEnterPressed: page.openSeries(card)
        Keys.onSpacePressed: page.playSeries(card)
        scale: theme.reducedMotion ? 1 : (nm.pressed ? 0.99 : 1)
        PosterTone { id: nextTone; source: nextTile.card.poster ? Fmt.fileUrl(nextTile.card.poster) : "" }
        // Like the series page band: the poster fills the right side top to bottom, the show's
        // colour is solid under the words and fades across into it. Every size follows the
        // width, so the height never feeds back into it.
        readonly property bool compact: width < theme.space(110)
        readonly property real pad: compact ? theme.space(4) : theme.space(5)
        readonly property real gap: theme.space(2)
        readonly property real artW: Math.round(width * (compact ? 0.4 : 0.36))
        // The words stop where the wash is still near solid
        readonly property real wordsW: width - pad * 2 - artW * 0.45
        readonly property bool hasArt: nextArt.status === Image.Ready
        // Tall enough that the poster shows as a poster, not a strip from its middle
        height: Math.max(theme.space(compact ? 30 : 36), Math.round(artW * 1.15), lead.height + gap + info.height + pad * 2)
        // Only with a poster to show: without one the band is the tone alone
        Corner {
            id: nextPoster
            visible: nextTile.hasArt
            anchors.right: parent.right
            width: nextTile.artW; height: parent.height
            radius: nextTile.radius
            fillItem: nextTile.hasArt ? nextArt : null
            Image {
                id: nextArt
                visible: false
                width: nextPoster.width; height: nextPoster.height
                source: nextTile.card.poster ? Fmt.fileUrl(nextTile.card.poster) : ""
                // the shared poster decode, a cache hit; Corner scales it to the box
                sourceSize: theme.posterDecodeSize
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
            }
        }
        // The wash: solid under the words, clearing to the right. Its radius rounds the two
        // right corners it shares with the card.
        Rectangle {
            visible: nextTile.hasArt
            anchors.right: parent.right
            width: nextTile.artW + theme.space(12); height: parent.height
            radius: nextTile.radius
            antialiasing: true
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0.0; color: nextTile.base }
                GradientStop { position: 0.25; color: nextTile.base }
                GradientStop { position: 0.5; color: Qt.rgba(nextTile.base.r, nextTile.base.g, nextTile.base.b, 0.8) }
                GradientStop { position: 0.8; color: Qt.rgba(nextTile.base.r, nextTile.base.g, nextTile.base.b, 0.2) }
                GradientStop { position: 1.0; color: Qt.rgba(nextTile.base.r, nextTile.base.g, nextTile.base.b, 0.0) }
            }
        }
        // The card's own edge (focus ring, hover line) above the art, so it runs all the way round
        Corner {
            z: 2
            anchors.fill: parent
            radius: nextTile.radius
            color: "transparent"
            borderColor: nextTile.borderColor
            borderWidth: nextTile.borderWidth
        }
        // The time left
        Column {
            id: lead
            x: nextTile.pad
            y: nextTile.pad
            width: nextTile.wordsW
            spacing: theme.space(1)
            Label { msg: "home-next-up"; role: "ledger"; color: nextTile.inkAccent }
            Flow {
                width: parent.width
                spacing: theme.space(2)
                Repeater {
                    model: nextTile.parts
                    Row {
                        required property var modelData
                        spacing: theme.space(0.75)
                        Label { id: fig; text: modelData.n; role: "display"; font.pointSize: theme.typeHeading * (nextTile.compact ? 1 : 1.2); font.variableAxes: ({ "wdth": 70, "wght": 560 }); color: nextTile.ink; font.features: { "tnum": 1 } }
                        Label { anchors.baseline: fig.baseline; text: modelData.unit; role: "title"; font.pointSize: theme.typeNormal; color: nextTile.inkDim }
                    }
                }
            }
        }
        // The series and when
        Column {
            id: info
            x: nextTile.pad
            y: lead.y + lead.height + nextTile.gap
            width: nextTile.wordsW
            spacing: theme.space(1)
            Label {
                width: parent.width
                text: nextTile.card.title || ""
                role: "title"
                font.pointSize: theme.typeNormal * (nextTile.compact ? 1 : 1.15)
                color: nextTile.ink
                wrapMode: Text.Wrap
                maximumLineCount: 3
                elide: Text.ElideRight
            }
            Label {
                width: parent.width
                text: {
                    var d = nextTile.drop
                    if (!d) return ""
                    var args = { season: d.season, span: words.episodeSpan(d.episode, d.through || 0),
                                 time: Qt.formatDateTime(new Date(d.at * 1000), words.t("home-next-time-format")) }
                    var seasoned = d.season && d.season !== 1
                    return d.predicted ? words.t(seasoned ? "home-next-when-season-estimated" : "home-next-when-estimated", args)
                        : words.t(seasoned ? "home-next-when-season" : "home-next-when", args)
                }
                role: "ui"
                color: nextTile.inkDim
                wrapMode: Text.Wrap
            }
        }
        MouseArea {
            Accessible.role: Accessible.Button
            Accessible.onPressAction: { nextTile.forceActiveFocus(); page.openSeries(nextTile.card) }
            Accessible.name: theme.nameNear(parent)
            id: nm
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: { nextTile.forceActiveFocus(); page.openSeries(nextTile.card) }
            onContainsMouseChanged: frame.hoverCard(nextTile, nextTile.card, containsMouse)
        }
        Component.onDestruction: frame.hoverCard(nextTile, null, false)
    }
    // One episode on the board: a poster thumb, then the time, the title and the episode. A
    // guessed time says so beside the time, a late one says it is not out yet, and how many of
    // the series wait on disk follows the episode. Opens the series, Space plays it.
    component BoardTile: Corner {
        id: tile
        property var entry: ({})
        readonly property bool gridStop: true
        objectName: "week-" + (card.id || 0) + (drop ? "-" + (drop.season ? drop.season + "x" : "") + drop.episode : "")
        readonly property var card: entry.card || ({})
        readonly property var drop: entry.drop || null
        readonly property bool overdue: entry.state === "overdue"
        readonly property bool guessed: entry.state === "future" && !!drop && !!drop.predicted
        // Out already today: it stays, counting up from when it aired
        readonly property bool aired: entry.aired === true
        readonly property real dim: aired && entry.watched ? 0.55 : 1
        readonly property int toWatch: page.toWatchOf(card)
        height: Math.max(theme.space(21), tileText.implicitHeight + theme.space(4))
        radius: theme.radiusMd
        smoothing: theme.cornerSmoothing
        color: tm.containsMouse ? theme.raised : theme.tile
        borderColor: tile.activeFocus ? theme.focusRing : tm.containsMouse ? theme.lineStrong : "transparent"
        borderWidth: 1
        activeFocusOnTab: true
        Keys.onReturnPressed: page.openSeries(card)
        Keys.onEnterPressed: page.openSeries(card)
        Keys.onSpacePressed: page.playSeries(card)
        Corner {
            id: thumb
            x: theme.space(2)
            anchors.verticalCenter: parent.verticalCenter
            height: theme.space(17); width: Math.round(height * 2 / 3)
            radius: theme.radiusSm
            color: theme.tileSunken
            opacity: tile.dim
            fillItem: thumbImg.status === Image.Ready ? thumbImg : null
            Image {
                id: thumbImg
                visible: false
                width: thumb.width; height: thumb.height
                source: tile.card.poster ? Fmt.fileUrl(tile.card.poster) : ""
                sourceSize: Qt.size(Math.ceil(thumb.width * 2), Math.ceil(thumb.height * 2))
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
            }
        }
        Column {
            id: tileText
            anchors.left: thumb.right; anchors.leftMargin: theme.space(3)
            anchors.right: parent.right; anchors.rightMargin: theme.space(3)
            anchors.verticalCenter: parent.verticalCenter
            spacing: theme.space(0.5)
            opacity: tile.dim
            Row {
                width: parent.width
                spacing: theme.space(1.5)
                Label {
                    id: when
                    text: !tile.drop ? words.t("home-no-date") : Qt.formatDateTime(new Date(tile.drop.at * 1000), tile.overdue ? words.t("home-day-time-format") : words.t("home-time-format"))
                    role: "ledger"
                    font.pointSize: theme.typeNormal
                    color: tile.overdue ? theme.red : tile.guessed ? theme.yellow : tile.drop ? theme.text : theme.textDim
                }
                Label {
                    anchors.baseline: when.baseline
                    width: Math.min(implicitWidth, parent.width - when.width - parent.spacing)
                    elide: Text.ElideRight
                    visible: tile.overdue || tile.guessed || tile.aired
                    text: tile.aired ? (tile.entry.watched ? words.t("home-tile-watched") : tile.entry.state === "downloading" ? words.t("home-tile-downloading") : words.t("home-tile-out", { ago: page.ago(tile.drop.at) }))
                        : tile.overdue ? words.t("home-tile-not-out") : words.t("home-tile-estimated")
                    role: "ledger"
                    color: tile.aired ? (tile.entry.state === "late" ? theme.red : tile.entry.state === "here" && !tile.entry.watched ? theme.accent : theme.textDim)
                        : tile.overdue ? theme.red : theme.yellow
                    opacity: tile.aired ? 1 : 0.8
                }
            }
            Label {
                width: parent.width
                text: tile.card.title || ""
                role: "ui"
                font.weight: 600
                color: tm.containsMouse ? theme.accent : theme.text
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
            }
            Row {
                width: parent.width
                spacing: theme.space(2)
                Label {
                    width: Math.min(implicitWidth, parent.width - (waits.visible ? waits.implicitWidth + parent.spacing : 0))
                    elide: Text.ElideRight
                    text: !tile.drop ? "" : tile.drop.season && tile.drop.season !== 1 ? words.t("home-tile-season-span", { season: tile.drop.season, span: words.episodeSpan(tile.drop.episode, tile.drop.through || 0) })
                        : words.episodeSpan(tile.drop.episode, tile.drop.through || 0)
                    visible: text !== ""
                    role: "ledger"
                    color: theme.textDim
                }
                Label { id: waits; visible: tile.toWatch > 0; text: words.t("home-tile-to-watch", { count: tile.toWatch }); role: "ledger"; color: theme.accent }
            }
        }
        MouseArea {
            Accessible.role: Accessible.Button
            Accessible.onPressAction: { tile.forceActiveFocus(); page.openSeries(tile.card) }
            Accessible.name: theme.nameNear(parent)
            id: tm
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: { tile.forceActiveFocus(); page.openSeries(tile.card) }
            onContainsMouseChanged: frame.hoverCard(tile, tile.card, containsMouse)
        }
        Component.onDestruction: frame.hoverCard(tile, null, false)
    }
    // A day on the board: its name and date over a rule, the accent's on today, then its
    // episodes. A day with nothing says so once, small. `across` is the narrow window's
    // layout: the name at the left of a row, the episodes flowing beside it, the rule on top.
    component BoardDay: Item {
        id: bday
        property var day: ({})
        property bool narrow: false
        property bool across: false
        readonly property real headW: across ? theme.space(28) : width
        // Across, as many tiles on a line as hold a readable title, sharing it evenly
        readonly property real tileMin: theme.space(64)
        readonly property int perLine: Math.max(1, Math.floor((list.width + list.spacing) / (tileMin + list.spacing)))
        readonly property real tileW: across ? (list.width - (perLine - 1) * list.spacing) / perLine : list.width
        height: across ? Math.max(rule.height + theme.space(3) + head.height, list.y + list.height) : list.y + list.height
        Rectangle {
            id: rule
            y: bday.across ? 0 : head.height + theme.space(2)
            width: parent.width
            height: bday.across && !bday.day.today ? 1 : 2
            radius: 1
            color: bday.day.today ? theme.accent : theme.line
        }
        Column {
            id: head
            y: bday.across ? rule.height + theme.space(3) : 0
            width: bday.headW - (bday.across ? theme.space(3) : 0)
            Label {
                width: parent.width
                text: bday.narrow && !bday.across ? bday.day.short : bday.day.name
                role: "title"
                font.pointSize: theme.typeNormal
                color: bday.day.today ? theme.accent : bday.day.later || bday.day.past ? theme.textDim : theme.text
                elide: Text.ElideRight
            }
            Label {
                width: parent.width
                text: (bday.narrow && !bday.across ? bday.day.shortDate : bday.day.date) || " "
                role: "ledger"
                color: theme.textDim
                elide: Text.ElideRight
            }
        }
        Flow {
            id: list
            x: bday.across ? bday.headW : 0
            y: bday.across ? rule.height + theme.space(3) : rule.y + rule.height + theme.space(2)
            width: bday.width - x
            spacing: theme.space(2)
            Label { visible: !(bday.day.items && bday.day.items.length); msg: "home-day-nothing"; role: "ledger"; color: theme.textFaint }
            Repeater {
                model: (bday.day.items || []).filter(function(e) { return e.aired })
                BoardTile { required property var modelData; entry: modelData; width: bday.tileW }
            }
            // Now, between what already came out today and what is still to come
            Item {
                objectName: "week-now-line"
                readonly property bool hasAired: (bday.day.items || []).some(function(e) { return e.aired })
                visible: bday.day.today && hasAired && bday.day.items.some(function(e) { return !e.aired })
                width: list.width
                height: nowLabel.implicitHeight
                Label { id: nowLabel; text: words.t("home-now", { time: Qt.formatTime(new Date(page.nowMs), words.t("home-time-format")) }); role: "ledger"; color: theme.accent }
                Rectangle {
                    anchors.left: nowLabel.right; anchors.leftMargin: theme.space(2)
                    anchors.right: parent.right
                    anchors.verticalCenter: nowLabel.verticalCenter
                    height: 1; color: theme.accent; opacity: 0.6
                }
            }
            NextTile {
                visible: !!bday.day.next
                entry: bday.day.next
                width: bday.across ? Math.min(list.width, bday.tileW * 2 + list.spacing) : bday.tileW
            }
            Repeater {
                model: (bday.day.items || []).filter(function(e) { return !e.next && !e.aired })
                BoardTile { required property var modelData; entry: modelData; width: bday.tileW }
            }
        }
    }

    // ---- A shelf: a heading over a grid of cards
    component Shelf: Column {
        id: shelfBox
        property string heading: ""
        property string fact: ""
        property var list: []
        property real minWidth: theme.posterWidth
        property int maxRows: 0
        property string moreText: words.t("action-see-all")
        signal more()
        readonly property int cols: Math.max(1, Math.floor((width + page.gap) / (minWidth + page.gap)))
        readonly property real cardWidth: Math.floor((width - page.gap * (cols - 1)) / cols)
        readonly property var shown: maxRows > 0 ? list.slice(0, cols * maxRows) : list
        visible: list.length > 0
        spacing: theme.space(3)
        Heading {
            text: shelfBox.heading
            fact: shelfBox.fact
            action: Button {
                visible: shelfBox.list.length > shelfBox.shown.length
                text: shelfBox.moreText
                small: true
                flat: true
                onClicked: shelfBox.more()
            }
        }
        Flow {
            width: shelfBox.width
            spacing: page.gap
            Repeater {
                model: shelfBox.shown
                Card {
                    required property var modelData
                    objectName: "card-" + modelData.id   // a --drive target
                    item: modelData
                    posterWidth: shelfBox.cardWidth
                    nowMs: page.nowMs
                    onOpened: page.openSeries(modelData)
                    onPlayed: page.playSeries(modelData)
                }
            }
        }
    }

    // ---- Continue (#1033): one card a series, the episode to go on with. A picture from that
    // episode at 16:9 (a plain face where the record has none), how far into it you stopped
    // as a thin bar along its foot, then the show and the episode. A click, Return or Space
    // plays it from where it stopped; the series is in the context menu, from a right click
    // or Shift+F10 / the Menu key on a focused card.
    function episodeLabel(ep) {
        if (!ep) return ""
        var n = words.episodeNumber(ep.number)
        var code = n === null ? "" : ep.season && ep.season > 1 ? words.t("home-card-episode-season", { season: ep.season, number: n })
                                                                 : words.t("home-card-episode", { number: n })
        var t = ep.title ? ep.title.trim() : ""
        return code && t ? words.t("home-card-episode-titled", { episode: code, title: t }) : code || t || words.episodeName("", ep.number)
    }
    function resumeFrac(e) {
        return e && e.partial && e.episode.resume && e.episode.resume.duration > 0 ? Math.min(1, e.episode.resume.position / e.episode.resume.duration) : 0
    }
    function minutesLeft(e) {
        return e && e.partial ? Math.max(1, Math.round((e.episode.resume.duration - e.episode.resume.position) / 60)) : 0
    }
    function playEntry(e) { if (e && e.episode) page.playFile(e.episode.file, e.card.title) }
    component EpisodeCard: Item {
        id: ecard
        property var entry: null
        // The hero's series: lit in the accent while it is up there
        property bool lead: false
        // Wholly inside the row, so the keys stop on it; a card gliding in or out, or one
        // under the right edge's shadow, is passed over
        property bool reachable: true
        readonly property bool gridStop: reachable
        objectName: "continue-" + (card.id || 0)
        readonly property var card: entry ? entry.card : ({})
        readonly property var ep: entry ? entry.episode : null
        // A picture from the episode and nothing else: its still, else a frame from its file.
        // Neither is a plain face, never the show's banner or poster.
        readonly property string art: ep ? (ep.still || ep.frame || "") : ""
        readonly property real frac: page.resumeFrac(entry)
        height: thumb.height + theme.space(2.5) + text.height
        activeFocusOnTab: reachable
        Keys.onReturnPressed: page.playEntry(entry)
        Keys.onEnterPressed: page.playEntry(entry)
        Keys.onSpacePressed: page.playEntry(entry)
        Accessible.role: Accessible.Button
        Accessible.name: (card.title || "") + ", " + page.episodeLabel(ep)
        Accessible.onPressAction: page.playEntry(entry)
        scale: theme.reducedMotion ? 1 : (em.pressed ? 0.985 : 1)
        Behavior on scale { NumberAnimation { duration: theme.motionFast } }
        Corner {
            id: thumb
            width: parent.width
            height: Math.round(width * 9 / 16)
            radius: theme.radiusTile
            smoothing: theme.cornerSmoothing
            color: theme.tileSunken
            fillItem: thumbImg.status === Image.Ready ? thumbImg : null
            Image {
                id: thumbImg
                visible: false
                width: thumb.width; height: thumb.height
                source: ecard.art ? Fmt.fileUrl(ecard.art) : ""
                sourceSize: Qt.size(Math.ceil(thumb.width * 2), Math.ceil(thumb.height * 2))
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
            }
            // A shade under the play mark and the time left, deeper while hovered
            Corner {
                anchors.fill: parent
                radius: thumb.radius
                smoothing: thumb.smoothing
                fillGradient: LinearGradient {
                    x1: 0; y1: 0; x2: 0; y2: thumb.height
                    GradientStop { position: 0; color: Qt.rgba(0, 0, 0, em.containsMouse ? 0.25 : 0) }
                    GradientStop { position: 0.55; color: Qt.rgba(0, 0, 0, em.containsMouse ? 0.3 : 0.05) }
                    GradientStop { position: 1; color: Qt.rgba(0, 0, 0, ecard.frac > 0 ? 0.55 : 0.2) }
                }
            }
            Rectangle {
                anchors.centerIn: parent
                width: theme.space(12); height: width; radius: width / 2
                color: Qt.rgba(0, 0, 0, 0.5)
                border.color: Qt.rgba(1, 1, 1, 0.35); border.width: 1
                // Not on a card under the right edge's shadow, where the row's arrow comes up
                opacity: (em.containsMouse && ecard.reachable) || ecard.activeFocus ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: theme.motionNormal } }
                Icon { anchors.centerIn: parent; anchors.horizontalCenterOffset: theme.space(0.25); glyph: "play"; size: theme.space(5); color: "#ffffff" }
            }
            Label {
                visible: ecard.frac > 0
                anchors.right: parent.right; anchors.rightMargin: theme.space(3)
                anchors.bottom: bar.top; anchors.bottomMargin: theme.space(2)
                text: words.t("home-hero-min-left", { count: page.minutesLeft(ecard.entry) })
                role: "ledger"
                color: "#f4f6f8"
            }
            // How far into the episode, along the frame's foot
            Rectangle {
                id: bar
                visible: ecard.frac > 0
                anchors.left: parent.left; anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: theme.space(3)
                height: theme.space(1)
                radius: height / 2
                color: Qt.rgba(1, 1, 1, 0.25)
                Rectangle {
                    width: Math.max(height, parent.width * ecard.frac); height: parent.height
                    radius: height / 2
                    color: theme.accent
                }
            }
        }
        // The frame's edge above the art: the focus ring, the accent ring of the hero's
        // series, the hover line
        Corner {
            anchors.fill: thumb
            radius: thumb.radius
            smoothing: thumb.smoothing
            borderColor: ecard.activeFocus ? theme.focusRing : ecard.lead ? theme.accent : em.containsMouse ? theme.lineStrong : "transparent"
            borderWidth: ecard.activeFocus || ecard.lead ? 2 : 1
            Behavior on borderColor { ColorAnimation { duration: theme.motionNormal } }
        }
        Column {
            id: text
            anchors.top: thumb.bottom; anchors.topMargin: theme.space(2.5)
            width: parent.width
            spacing: theme.space(0.5)
            Label {
                width: parent.width
                text: ecard.card.title || ""
                role: "ui"
                font.weight: 600
                color: em.containsMouse || ecard.lead ? theme.accent : theme.text
                elide: Text.ElideRight
            }
            Label {
                width: parent.width
                text: page.episodeLabel(ecard.ep)
                role: "ui"
                font.pointSize: theme.typeSmall
                color: theme.textDim
                elide: Text.ElideRight
            }
        }
        MouseArea {
            id: em
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: { ecard.forceActiveFocus(); page.playEntry(ecard.entry) }
            onContainsMouseChanged: frame.hoverCard(ecard, ecard.card, containsMouse || ecard.activeFocus)
        }
        // The context menu reads the card under the pointer, or the focused one from the keys
        onActiveFocusChanged: if (!em.containsMouse) frame.hoverCard(ecard, ecard.card, activeFocus)
        Component.onDestruction: frame.hoverCard(ecard, null, false)
    }
    // Continue as one row (#1468): as many cards as the width holds, the hero's series at the
    // left and lit. Each step of the hero glides the row one card along; with more series
    // than fit, the row has no ends: the card that leaves at the left comes round again at
    // the right, and a heavy shadow over the right edge says more follow. With all of them in
    // view the row stands still and only the light moves. Nothing in it takes the wheel, so a
    // scroll over it moves the page.
    component ContinueRow: Column {
        id: row
        property var entries: []
        // The hero's place in Continue
        property int lead: 0
        // A card under the pointer or the focus holds the hero, as the hero itself does
        readonly property bool held: rowHover.hovered || focusInside
        readonly property bool focusInside: {
            for (var f = row.Window.activeFocusItem; f; f = f.parent) if (f === row) return true
            return false
        }
        readonly property real minWidth: theme.space(76)
        readonly property int count: entries.length
        readonly property int cols: Math.max(1, Math.floor((width + page.gap) / (minWidth + page.gap)))
        readonly property real cardWidth: Math.floor((width - page.gap * (cols - 1)) / cols)
        readonly property real pitch: cardWidth + page.gap
        readonly property bool loops: count > cols
        // The cards' height, which the track takes, since they sit in it by hand
        property real cardH: 0
        // Where the row stands, counted in cards and never wrapped: `target` is where it is
        // headed and `pos` follows it, between two whole numbers while it glides
        property int target: 0
        property real pos: 0
        function snap() {
            glide.stop()
            target = lead
            pos = lead
        }
        // The card standing at `place` once the row is still, by its index
        function cardAt(place) {
            var n = count
            return n ? cards.itemAt(((target + place) % n + n) % n) : null
        }
        // The focused card's place, or -1 with the focus elsewhere
        function focusedPlace() {
            for (var i = 0; i < cards.count; i++) {
                var c = cards.itemAt(i)
                if (c && c.activeFocus) return Math.round(c.place)
            }
            return -1
        }
        // A step of the hero's glides the row by one. A focused card that would glide out of
        // reach hands the focus to the card that takes its place.
        function follow() {
            if (reloading) return
            if (!loops || count === 0) { snap(); return }
            var at = ((target % count) + count) % count
            var d = ((lead - at) % count + count) % count
            if (d > count / 2) d -= count
            if (d === 0) return
            var had = focusedPlace()
            if (Math.abs(d) !== 1 || theme.reducedMotion) snap()
            else {
                target += d
                glide.from = pos
                glide.to = target
                glide.restart()
            }
            if (had >= 0) {
                var to = cardAt(had)
                if (to) to.forceActiveFocus()
            }
        }
        // A reload rebuilds every card in the new order, so the row snaps to the hero's place
        // once the reload has settled, and a lead change that came with it never glides
        property bool reloading: false
        function settle() { reloading = false; snap() }
        onEntriesChanged: { reloading = true; Qt.callLater(row.settle) }
        onLeadChanged: follow()
        onLoopsChanged: snap()
        // Left and Right walk the cards in the order they stand on screen, which is not their
        // order in the model once the row has come round
        function stepFocus(from, by) {
            if (!loops) return false
            var to = cardAt(Math.round(from.place) + by)
            if (to && to.reachable) to.forceActiveFocus()
            return true
        }
        // Tab and Shift+Tab the same way, leaving the row from its ends for whatever the focus
        // chain holds past the row's first or last card in the model
        function tabFocus(from, by) {
            if (!loops) return false
            var to = cardAt(Math.round(from.place) + by)
            if (to && to.reachable) { to.forceActiveFocus(); return true }
            // Forward from the last reachable card in the model, back from the first
            var edge = null
            for (var i = 0; i < cards.count; i++) {
                var c = cards.itemAt(by > 0 ? i : cards.count - 1 - i)
                if (c && c.reachable) edge = c
            }
            var out = edge ? edge.nextItemInFocusChain(by > 0) : null
            if (out) out.forceActiveFocus(by > 0 ? Qt.TabFocusReason : Qt.BacktabFocusReason)
            return true
        }
        NumberAnimation {
            id: glide
            target: row; property: "pos"
            duration: 380
            easing.type: Easing.InOutCubic
        }
        // Steps the row and the hero together, as the hero's own arrows do
        signal step(int by)
        visible: entries.length > 0
        spacing: theme.space(3)
        HoverHandler { id: rowHover }
        Heading {
            text: words.t("home-continue")
            fact: words.t("home-continue-fact", { count: row.entries.length })
        }
        // The track runs out to the window's edges, so a card glides in from the edge and out
        // past it rather than vanishing at the column's side
        Item {
            id: track
            objectName: "home-continue-track"
            width: row.width
            height: row.count ? row.cardH : 0
            Item {
                id: lane
                x: -scroll.gutter
                y: -theme.space(1)
                width: scroll.width
                height: parent.height + theme.space(2)
                clip: true
                Repeater {
                    id: cards
                    model: row.entries
                    EpisodeCard {
                        id: slot
                        required property var modelData
                        required property int index
                        entry: modelData
                        width: row.cardWidth
                        lead: index === ((row.lead % Math.max(1, row.count)) + Math.max(1, row.count)) % Math.max(1, row.count)
                        // Its place counted in cards from the row's left. The card that just
                        // left at the left keeps going off that side while a step glides on.
                        readonly property real place: {
                            if (!row.loops) return index
                            var n = row.count, d = ((index - row.target) % n + n) % n
                            if (row.pos < row.target && d === n - 1) d = -1
                            return d - (row.pos - row.target)
                        }
                        x: scroll.gutter + place * row.pitch
                        y: theme.space(1)
                        visible: x + width > 0 && x < lane.width
                        // Inside the row, and clear of the right edge's shadow when it loops
                        reachable: place > -0.01 && place < row.cols - (row.loops ? 1.99 : 0.99)
                        Keys.onLeftPressed: function(event) { event.accepted = row.stepFocus(slot, -1) }
                        Keys.onRightPressed: function(event) { event.accepted = row.stepFocus(slot, 1) }
                        Keys.onTabPressed: function(event) { event.accepted = row.tabFocus(slot, 1) }
                        Keys.onBacktabPressed: function(event) { event.accepted = row.tabFocus(slot, -1) }
                        onHeightChanged: if (index === 0) row.cardH = height
                        Component.onCompleted: if (index === 0) row.cardH = height
                    }
                }
                // More to the right: the last card in view sinks into the page
                Rectangle {
                    objectName: "home-continue-more"
                    visible: row.loops
                    z: 2
                    anchors.right: parent.right
                    width: scroll.gutter + row.cardWidth
                    height: parent.height
                    gradient: Gradient {
                        orientation: Gradient.Horizontal
                        GradientStop { position: 0; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0) }
                        GradientStop { position: 0.25; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.55) }
                        GradientStop { position: 0.55; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.9) }
                        GradientStop { position: 0.8; color: theme.bg }
                        GradientStop { position: 1; color: theme.bg }
                    }
                }
            }
            // The row's edges, while it loops: a pointer resting on the shadowed right end or
            // at the very left brings up an arrow there, which steps the row and the hero
            // together. Moving off the edge puts it away.
            HoverHandler {
                id: edgeHover
                enabled: row.loops
                onPointChanged: {
                    var x = point.position.x
                    var side = !hovered ? "" : x >= row.width - row.cardWidth ? "right" : x < theme.space(16) ? "left" : ""
                    if (side !== edges.shown) edges.shown = ""
                    edges.pending = side
                    if (side !== "") edgeRest.restart(); else edgeRest.stop()
                }
                onHoveredChanged: if (!hovered) { edgeRest.stop(); edges.shown = "" }
            }
            QtObject {
                id: edges
                property string shown: ""
                property string pending: ""
            }
            Timer { id: edgeRest; interval: 300; onTriggered: edges.shown = edges.pending }
            Repeater {
                model: ["left", "right"]
                Button {
                    id: edgeArrow
                    required property string modelData
                    readonly property bool on: row.loops && edges.shown === modelData
                    objectName: "home-continue-" + modelData
                    z: 3
                    x: modelData === "left" ? theme.space(3) : row.width - width - theme.space(3)
                    y: Math.round(row.cardWidth * 9 / 32 - height / 2)
                    width: height
                    ghost: true
                    takesFocus: false
                    icon: modelData === "left" ? "chevron-left" : "chevron-right"
                    accessibleName: words.t(modelData === "left" ? "home-hero-previous" : "home-hero-next")
                    opacity: on ? 1 : 0
                    visible: opacity > 0
                    enabled: on
                    Behavior on opacity { NumberAnimation { duration: theme.motionNormal } }
                    onClicked: row.step(modelData === "left" ? -1 : 1)
                }
            }
        }
    }

    RecommendedHover {
        id: recHover
        onOpened: function(s) { page.openShow(s) }
    }
    Connections { target: scroll; function onContentYChanged() { recHover.close() } }

    PageScroll {
        id: scroll
        anchors.fill: parent
        visible: !page.libraryEmpty

        // ---- The hero (#1464, #1468): the Continue series one at a time, every one in the same
        // shape. The show's backdrop (#1475) runs to the window's edges under a quiet scrim and
        // fades into the page (with none, its poster blurred in its place); low on it sit the poster,
        // the logo or the title, the episode, two lines of the show's synopsis and the two
        // actions. The poster is a picture and nothing more: playing is the button's job and
        // Continue's. A show with no logo makes its poster the larger centre piece, the title
        // beside it. The arrows step through the series, and Continue steps with them; after a
        // stretch with no input it moves on by itself, slowly, and never while the pointer is
        // on it or on Continue. Each step crossfades.
        Item {
            id: hero
            objectName: "home-hero"
            visible: count > 0
            // Under the rest of the column, so the art can run up behind the page's head
            z: -1
            width: parent.width
            readonly property int count: heroIds.count
            // The series shown and the one fading out under it, by id, so a reload that moves
            // them in the list moves nothing on screen; `index` is the shown one's place, for
            // the steps and for Continue's row
            property int shownId: -1
            property int prevId: -1
            property int index: 0
            property real fade: 1
            // The poster's height, one size for every show; a short window gives the hero
            // less, so Continue still shows under it. A show with a logo fits its words into
            // the same height, the logo's top on the poster's top.
            readonly property real frameH: Math.round(Math.min(theme.space(85), page.height * 0.325))
            // The poster drops out on a narrow window, the words keep the width
            readonly property bool framed: width >= theme.space(200)
            readonly property real topPad: Math.round(Math.max(theme.space(16), Math.min(theme.space(40), page.height * 0.13)))
            readonly property real tail: theme.space(4)
            // The tallest slide sets the height, so a step never moves Continue
            readonly property real contentH: {
                var h = framed ? frameH : 0
                for (var i = 0; i < slides.count; i++) {
                    var s = slides.itemAt(i)
                    if (s) h = Math.max(h, s.blockH)
                }
                return Math.ceil(h)
            }
            height: visible ? topPad + contentH + tail : 0
            function indexOf(id) {
                for (var i = 0; i < heroIds.count; i++) if (heroIds.get(i).sid === id) return i
                return -1
            }
            // After a reload: the series shown stays shown, and a fade under way runs on, unless
            // its series left the list
            function afterSync() {
                if (indexOf(shownId) < 0) {
                    fadeIn.stop()
                    fade = 1
                    prevId = -1
                    shownId = heroIds.count ? heroIds.get(0).sid : -1
                }
                if (prevId >= 0 && indexOf(prevId) < 0) prevId = -1
                index = Math.max(0, indexOf(shownId))
            }
            // A step asked for while a crossfade runs waits for it to end, so the picture never
            // jumps from a half faded slide
            property int pendingId: -1
            function show(i) {
                if (count < 2) return
                i = (i % count + count) % count
                var id = heroIds.get(i).sid
                lastInput = Date.now()
                if (fadeIn.running) { pendingId = id === shownId ? -1 : id; return }
                if (id === shownId) return
                // Focus on the outgoing slide goes to the incoming one's button, since the
                // outgoing slide is disabled and would drop it
                var old = slides.itemAt(index)
                var refocus = false
                for (var f = hero.Window.activeFocusItem; f; f = f.parent) if (f === old) { refocus = true; break }
                prevId = shownId
                shownId = id
                index = i
                if (refocus) slides.itemAt(i).focusResume()
                if (theme.reducedMotion) { fadeIn.stop(); fade = 1; prevId = -1 }
                else fadeIn.restart()
            }
            NumberAnimation {
                id: fadeIn
                target: hero; property: "fade"
                from: 0; to: 1
                duration: 900
                easing.type: Easing.InOutQuad
                onFinished: {
                    hero.prevId = -1
                    var next = hero.indexOf(hero.pendingId)
                    hero.pendingId = -1
                    if (next >= 0) hero.show(next)
                }
            }
            // On by itself after eight seconds with no input, and never while the pointer
            // rests on the hero or on Continue, the keyboard focus is inside either, or the
            // hero is scrolled away. Any step, a pointer leaving and a scroll start the eight
            // seconds again.
            readonly property int idleMs: 8000
            property real lastInput: Date.now()
            readonly property bool focusInside: {
                for (var f = hero.Window.activeFocusItem; f; f = f.parent) if (f === hero) return true
                return false
            }
            readonly property bool held: heroHover.hovered || focusInside || continueRow.held
            onHeldChanged: lastInput = Date.now()
            Connections { target: scroll; function onContentYChanged() { hero.lastInput = Date.now() } }
            Timer {
                interval: 500
                repeat: true
                running: hero.count > 1 && page.visible && !hero.held && scroll.contentY < hero.height
                onTriggered: if (Date.now() - hero.lastInput >= hero.idleMs) hero.show(hero.index + 1)
            }
            HoverHandler { id: heroHover }

            Repeater {
                id: slides
                model: heroIds
                Item {
                    id: slide
                    required property int sid
                    required property int index
                    readonly property var entry: page.heroEntries[sid] || null
                    readonly property bool current: sid === hero.shownId
                    readonly property bool leaving: sid === hero.prevId
                    // Only what is on screen, or next up either way, holds its art in memory
                    readonly property bool loaded: current || leaving
                        || (hero.count > 1 && (index === (hero.index + 1) % hero.count || index === (hero.index - 1 + hero.count) % hero.count))
                    readonly property var c: entry ? entry.card : ({})
                    readonly property var d: entry ? entry.detail : null
                    readonly property var ep: entry ? entry.episode : null
                    // The layout follows the record alone, so a logo that is slow or fails to
                    // load never moves anything
                    readonly property bool hasLogo: !!(d && d.logo)
                    readonly property real posterH: hero.frameH
                    // With a logo the words take the poster's height whatever the window;
                    // without one, the words' own height
                    readonly property real blockH: Math.max(hero.framed || hasLogo ? posterH : 0, hasLogo ? 0 : lower.implicitHeight)
                    // The core's pick (#1475): the wider of the show's background and its
                    // banner, else nothing and the poster wash below
                    readonly property string backdrop: d && d.backdrop ? d.backdrop : ""
                    readonly property string poster: c.poster || ""
                    function focusResume() { resumeButton.forceActiveFocus() }
                    // The incoming slide fades in over the outgoing one, which stays whole
                    // under it, so the art never dips to the page between the two
                    z: current ? 2 : leaving ? 1 : 0
                    opacity: current || leaving ? 1 : 0
                    enabled: current
                    width: hero.width
                    height: hero.height
                    PosterTone { id: tone; source: slide.loaded && slide.poster ? Fmt.fileUrl(slide.poster) : "" }
                    readonly property color bandBase: theme.bandBase(tone.tone, slide.poster ? 0.5 : 0.25)
                    readonly property color inkAccent: theme.bandAccent(tone.tone, bandBase)
                    readonly property color ink: "#f4f6f8"
                    readonly property color inkDim: Qt.rgba(1, 1, 1, 0.72)

                    Item {
                        id: back
                        x: -scroll.gutter
                        y: -(hero.y + theme.space(7))
                        width: scroll.width
                        height: hero.height - y
                        clip: true
                        opacity: slide.current ? hero.fade : 1
                        // Faded as one picture, or the scrims would fade apart from the art
                        layer.enabled: opacity < 1
                        readonly property bool hasArt: art.visible || soft.visible
                        Rectangle { anchors.fill: parent; color: slide.bandBase }
                        Image {
                            id: art
                            visible: slide.backdrop !== "" && status === Image.Ready
                            anchors.fill: parent
                            source: slide.loaded && slide.backdrop ? Fmt.fileUrl(slide.backdrop) : ""
                            // Decoded at the width it is drawn, not the picture's own
                            sourceSize.width: Math.ceil(back.width * Screen.devicePixelRatio)
                            fillMode: Image.PreserveAspectCrop
                            // A banner strip keeps its top; a 16:9 background crops about its middle
                            verticalAlignment: slide.d && slide.backdrop === slide.d.banner ? Image.AlignTop : Image.AlignVCenter
                            asynchronous: true
                        }
                        // No backdrop: the poster, decoded tiny and drawn large, so it reads as
                        // its colours and nothing sharp
                        Image {
                            id: soft
                            visible: slide.backdrop === "" && slide.poster !== "" && status === Image.Ready
                            anchors.fill: parent
                            source: slide.loaded && slide.backdrop === "" && slide.poster ? Fmt.fileUrl(slide.poster) : ""
                            sourceSize: Qt.size(12, 18)
                            fillMode: Image.PreserveAspectCrop
                            smooth: true
                            opacity: 0.8
                            asynchronous: true
                        }
                        Rectangle { visible: back.hasArt; anchors.fill: parent; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.35) }
                        Rectangle {
                            visible: back.hasArt
                            anchors.fill: parent
                            gradient: Gradient {
                                orientation: Gradient.Horizontal
                                GradientStop { position: 0; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.7) }
                                GradientStop { position: 0.5; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.3) }
                                GradientStop { position: 0.85; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0) }
                            }
                        }
                        Rectangle {
                            id: fade
                            anchors.fill: parent
                            readonly property real start: Math.max(0.05, (hero.topPad - theme.space(24) - back.y) / back.height)
                            readonly property real solid: Math.min(0.98, (hero.topPad + hero.contentH * 0.8 - back.y) / back.height)
                            gradient: Gradient {
                                GradientStop { position: 0; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0) }
                                GradientStop { position: fade.start; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0) }
                                GradientStop { position: fade.start + (fade.solid - fade.start) * 0.5; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 0.6) }
                                GradientStop { position: fade.solid; color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, back.hasArt ? 0.97 : 1) }
                                GradientStop { position: 1; color: theme.bg }
                            }
                        }
                    }

                    Item {
                        anchors.fill: parent
                        // The outgoing words clear quickly, so two never read on top of each other
                        opacity: slide.current ? hero.fade : Math.max(0, 1 - hero.fade * 2)
                        layer.enabled: opacity > 0 && opacity < 1

                        // The poster, on the hero's foot line. Nothing happens on it.
                        Corner {
                            id: posterBox
                            objectName: slide.current ? "home-hero-poster" : ""
                            visible: hero.framed
                            x: 0
                            y: hero.topPad + hero.contentH - height
                            height: slide.posterH
                            width: Math.round(height * 2 / 3)
                            radius: theme.radiusTile
                            smoothing: theme.cornerSmoothing
                            color: Qt.rgba(0, 0, 0, 0.3)
                            fillItem: posterImg.status === Image.Ready ? posterImg : null
                            Image {
                                id: posterImg
                                visible: false
                                width: posterBox.width; height: posterBox.height
                                source: slide.loaded && slide.poster ? Fmt.fileUrl(slide.poster) : ""
                                sourceSize: Qt.size(Math.ceil(posterBox.width * 2), Math.ceil(posterBox.height * 2))
                                fillMode: Image.PreserveAspectCrop
                                asynchronous: true
                            }
                            Corner {
                                anchors.fill: parent
                                radius: posterBox.radius
                                smoothing: posterBox.smoothing
                                borderColor: Qt.rgba(1, 1, 1, 0.12)
                                borderWidth: 1
                            }
                        }

                        // The words end on the poster's foot, so the actions line up with it.
                        // With a logo they span the poster's height: the logo fills what the
                        // lines under it leave, its top on the poster's top.
                        Item {
                            id: wordsBox
                            x: hero.framed ? posterBox.width + theme.space(8) : 0
                            height: slide.hasLogo ? hero.contentH : lower.implicitHeight
                            y: hero.topPad + hero.contentH - height
                            width: hero.width - x - (stepper.visible ? stepper.width + theme.space(6) : 0)

                            // The logo, from its top left corner, as tall as the room allows and
                            // no wider than a bound, so every logo starts on one line
                            Item {
                                id: logoArea
                                visible: slide.hasLogo
                                width: Math.min(parent.width, theme.space(140))
                                height: Math.max(0, wordsBox.height - lower.implicitHeight - theme.space(3))
                                readonly property bool logoShown: logo.status === Image.Ready
                                Image {
                                    id: logo
                                    objectName: slide.current ? "home-hero-logo" : ""
                                    visible: logoArea.logoShown
                                    anchors.fill: parent
                                    source: slide.loaded && slide.hasLogo ? Fmt.fileUrl(slide.d.logo) : ""
                                    // Decoded no larger than the box it fits into
                                    sourceSize: Qt.size(Math.ceil(width * Screen.devicePixelRatio), Math.ceil(height * Screen.devicePixelRatio))
                                    fillMode: Image.PreserveAspectFit
                                    horizontalAlignment: Image.AlignLeft
                                    verticalAlignment: Image.AlignTop
                                    asynchronous: true
                                    smooth: true
                                    mipmap: true
                                }
                                MouseArea {
                                    visible: logoArea.logoShown
                                    width: logo.paintedWidth
                                    height: logo.paintedHeight
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: page.openSeries(slide.c)
                                    Accessible.role: Accessible.Button
                                    Accessible.name: words.t("home-hero-open", { title: slide.c.title || "" })
                                    Accessible.onPressAction: page.openSeries(slide.c)
                                }
                            }

                            Column {
                                id: lower
                                anchors.bottom: parent.bottom
                                width: parent.width
                                spacing: theme.space(3)
                                // The title: in the display cut where there is no logo, a
                                // smaller line under the logo where there is one
                                Label {
                                    id: titleLabel
                                    objectName: slide.current ? "home-hero-title" : ""
                                    width: parent.width
                                    text: slide.c.title || ""
                                    role: "display"
                                    font.pointSize: slide.hasLogo ? theme.typeHeading : theme.typeDisplay * (hero.framed ? 1.2 : 1)
                                    color: slide.ink
                                    wrapMode: slide.hasLogo ? Text.NoWrap : Text.Wrap
                                    maximumLineCount: slide.hasLogo ? 1 : 2
                                    elide: Text.ElideRight
                                    lineHeight: 0.92
                                    MouseArea {
                                        width: Math.min(titleLabel.implicitWidth, titleLabel.width)
                                        height: parent.height
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: page.openSeries(slide.c)
                                        Accessible.role: Accessible.Button
                                        Accessible.name: words.t("home-hero-open", { title: slide.c.title || "" })
                                        Accessible.onPressAction: page.openSeries(slide.c)
                                    }
                                }
                                // The episode, then what the show is about, the same two lines
                                // for every show
                                Column {
                                    width: parent.width
                                    spacing: theme.space(1.5)
                                    Label {
                                        objectName: slide.current ? "home-hero-episode" : ""
                                        width: parent.width
                                        text: page.episodeLabel(slide.ep)
                                        role: "title"
                                        font.pointSize: theme.typeNormal * 1.15
                                        color: slide.ink
                                        elide: Text.ElideRight
                                    }
                                    Label {
                                        objectName: slide.current ? "home-hero-synopsis" : ""
                                        readonly property string body: slide.d && slide.d.synopsis ? slide.d.synopsis.split(/\n\s*\n/)[0].trim() : ""
                                        visible: body !== ""
                                        width: Math.min(parent.width, theme.space(170))
                                        text: body
                                        role: "ui"
                                        color: slide.inkDim
                                        wrapMode: Text.Wrap
                                        maximumLineCount: 2
                                        elide: Text.ElideRight
                                    }
                                }
                                Row {
                                    spacing: theme.space(2.5)
                                    Button {
                                        id: resumeButton
                                        objectName: slide.current ? "home-resume" : ""
                                        // The top of the page's vertical key order: Down from here
                                        // lands on the first card, Up from the first row comes back
                                        readonly property bool gridStop: slide.current
                                        primary: true
                                        tint: slide.inkAccent
                                        icon: "play"
                                        // One wording for every show, whether the episode was
                                        // stopped partway or not started yet
                                        readonly property var number: slide.ep ? words.episodeNumber(slide.ep.number) : null
                                        text: !slide.ep ? "" : number === null ? words.t("series-play-continue")
                                              : words.t("series-play-continue-episode", { number: number })
                                        onClicked: page.playEntry(slide.entry)
                                    }
                                    Button {
                                        ghost: true
                                        icon: "layers"
                                        msg: "action-open-series"
                                        onClicked: page.openSeries(slide.c)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // The steps, back and on, at the foot of the band, level with the actions. Continue's
            // lit card says where in the turn the hero is.
            Row {
                id: stepper
                objectName: "home-hero-steps"
                visible: hero.count > 1
                z: 3
                anchors.right: parent.right
                y: hero.topPad + hero.contentH - height
                spacing: theme.space(3)
                Button {
                    objectName: "home-hero-previous"
                    anchors.verticalCenter: parent.verticalCenter
                    // A click leaves the focus where it was, so a clicked arrow does not hold
                    // the cycle; Tab still reaches it
                    takesFocus: false
                    activeFocusOnTab: true
                    Accessible.focusable: true
                    width: height
                    ghost: true
                    icon: "chevron-left"
                    accessibleName: words.t("home-hero-previous")
                    onClicked: hero.show(hero.index - 1)
                    Tooltip { msg: "home-hero-previous" }
                }
                Button {
                    objectName: "home-hero-next"
                    anchors.verticalCenter: parent.verticalCenter
                    takesFocus: false
                    activeFocusOnTab: true
                    Accessible.focusable: true
                    width: height
                    ghost: true
                    icon: "chevron-right"
                    accessibleName: words.t("home-hero-next")
                    onClicked: hero.show(hero.index + 1)
                    Tooltip { msg: "home-hero-next" }
                }
            }
        }

        Item { width: 1; height: hero.visible ? theme.space(2) : 0 }

        // ---- Continue: every series with an episode to go on with, the hero's among them (#719),
        // stepping along with the hero (#1468)
        ContinueRow {
            id: continueRow
            objectName: "home-continue"
            width: parent.width
            entries: page.continueList
            lead: hero.index
            onStep: function(by) { hero.show(hero.index + by) }
        }
        Item { width: 1; height: continueRow.visible ? theme.space(4) : 0 }

        Shelf {
            objectName: "home-downloaded"
            width: parent.width
            heading: words.t("home-downloaded")
            fact: page.downloadedList.length ? words.t("home-downloaded-fact", { count: page.downloadedList.length }) : ""
            list: page.downloadedList
            maxRows: 1
            onMore: frame.go("library", { sort: "Downloaded", arrange: "None" })
        }
        Item { width: 1; height: page.downloadedList.length ? theme.space(4) : 0 }

        // ---- The week (#326, #352): Out now first, the posters of what aired and waits to be
        // watched (a click plays it; one not downloaded opens the series to get it); then the
        // board of the days ahead, a column a day. The board takes the whole row: days with
        // episodes share it, empty days keep a narrow column, and a window too narrow for all
        // of them wraps the days onto a second line.
        Column {
            id: weekBox
            objectName: "home-week"
            visible: page.airingRows.length > 0
            width: parent.width
            spacing: theme.space(3)
            readonly property var split: page.weekSplit
            readonly property int outCount: split.out.length
            readonly property bool hasAhead: page.board.some(function(d) { return d.items.length > 0 })
            // Still to come, leaving out what aired today and stays on Today
            readonly property bool hasUpcoming: page.board.some(function(d) { return d.items.some(function(e) { return !e.aired }) })
            readonly property real cardW: theme.posterWidth
            Heading {
                text: words.t("home-week"); fact: page.weekFact
                action: Row {
                    spacing: theme.space(2)
                    Button {
                        objectName: "home-week-hide-empty"
                        visible: weekBox.hasAhead
                        flat: true; small: true
                        text: page.hideEmptyDays ? words.t("home-week-show-empty") : words.t("home-week-hide-empty")
                        icon: page.hideEmptyDays ? "eye" : "eye-off"
                        onClicked: page.setHideEmptyDays(!page.hideEmptyDays)
                    }
                    // The feed left the rail (#1012): its way in is here, beside the week it extends
                    Button { objectName: "home-week-feed"; flat: true; small: true; msg: "home-week-feed"; icon: "rss"; onClicked: frame.nav.openRail("feed", frame.leavingScroll()) }
                    Button { objectName: "home-week-airing"; flat: true; small: true; msg: "home-week-airing"; icon: "chevron-right"; onClicked: frame.go("airing") }
                }
            }
            // Out now
            Column {
                objectName: "home-week-out"
                visible: weekBox.outCount > 0
                width: parent.width
                spacing: theme.space(3)
                WeekLabel { text: words.t("home-week-out"); fact: words.t("count-episodes", { count: weekBox.outCount }) }
                Flow {
                    width: parent.width
                    spacing: page.gap
                    Repeater {
                        model: weekBox.split.out
                        Card {
                            required property var modelData
                            objectName: "week-out-" + modelData.card.id
                            item: modelData.card
                            posterWidth: weekBox.cardW
                            nowMs: page.nowMs
                            metaOverride: page.outLine(modelData)
                            onOpened: modelData.state === "late" ? page.openSeries(modelData.card) : page.playSeries(modelData.card)
                            onPlayed: page.playSeries(modelData.card)
                        }
                    }
                }
            }
            Item { visible: weekBox.outCount > 0 && weekBox.hasAhead; width: 1; height: theme.space(3) }
            WeekLabel {
                visible: weekBox.outCount > 0 && weekBox.hasUpcoming
                text: words.t("home-week-coming-up")
                // The heading counts them already when it has nothing more pressing to say
                fact: { var n = weekBox.split.ahead.length + (weekBox.split.next ? 1 : 0); return n && !page.weekFactCounts ? words.t("count-episodes", { count: n }) : "" }
            }
            // The board
            Flow {
                id: board
                objectName: "home-week-board"
                visible: weekBox.hasAhead
                width: parent.width
                spacing: across ? theme.space(4) : theme.space(3)
                readonly property real narrowW: theme.space(20)
                readonly property real minDayW: theme.space(52)
                // The days drawn: every one, or with #688's toggle on only those with episodes
                readonly property var shown: page.hideEmptyDays ? page.board.filter(function(d) { return d.items.length > 0 }) : page.board
                readonly property int busy: shown.filter(function(d) { return d.items.length > 0 }).length
                readonly property int idle: shown.length - busy
                readonly property real dayW: busy ? Math.floor((width - idle * narrowW - (shown.length - 1) * theme.space(3)) / busy) : 0
                // Too narrow for a column a day: a row a day instead, the days down the page
                readonly property bool across: dayW < minDayW
                // Its days are lanes for the arrow keys (GridKeys)
                readonly property bool gridBoard: true
                readonly property bool gridAcross: across
                Repeater {
                    model: board.shown
                    BoardDay {
                        required property var modelData
                        day: modelData
                        narrow: modelData.items.length === 0
                        across: board.across
                        width: across ? board.width : narrow ? board.narrowW : board.dayW
                    }
                }
            }
        }
        Item { width: 1; height: page.airingRows.length ? theme.space(4) : 0 }

        // ---- Recommended for you (#779): ranked from the user's own list, best first. The
        // switch picks what the row holds: every show still to watch, only what the library
        // has, or only what is on a list. Holding the pointer on a tile opens its hover card.
        Column {
            id: recBox
            objectName: "home-recommended"
            width: parent.width
            spacing: theme.space(3)
            visible: Door.preferences.browse_show_recommended !== false && !!page.recommended && page.recommended.counts.all > 0
            readonly property int cols: Math.max(1, Math.floor((width + page.gap) / (theme.posterWidth + page.gap)))
            readonly property real cardWidth: Math.floor((width - page.gap * (cols - 1)) / cols)
            readonly property var shown: page.recList.slice(0, cols)
            readonly property var counts: page.recommended ? page.recommended.counts : ({ all: 0, in_library: 0, on_list: 0 })
            Heading {
                text: words.t("home-recommended")
                fact: words.t("home-recommended-fact")
                action: Row {
                    spacing: theme.space(3)
                    Seg {
                        objectName: "home-rec-scope"
                        anchors.verticalCenter: parent.verticalCenter
                        small: true
                        accessibleName: words.t("home-recommended-scope")
                        options: [{ text: words.t("home-recommended-all"), name: "all", count: recBox.counts.all },
                                  { text: words.t("home-recommended-in-library"), name: "library", count: recBox.counts.in_library },
                                  { text: words.t("home-recommended-on-list"), name: "list", count: recBox.counts.on_list }]
                        index: Math.max(0, page.recScopes.indexOf(page.recScope))
                        onPicked: function(i) { page.setRecScope(page.recScopes[i]) }
                    }
                    Button {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: page.recList.length > recBox.shown.length
                        msg: "action-see-all"
                        small: true
                        flat: true
                        onClicked: frame.go("recommended", {})
                    }
                }
            }
            Flow {
                width: recBox.width
                spacing: page.gap
                visible: recBox.shown.length > 0
                Repeater {
                    model: recBox.shown
                    RecommendedCard {
                        id: recTile
                        required property var modelData
                        objectName: "rec-" + modelData.anilist_id
                        rec: modelData
                        posterWidth: recBox.cardWidth
                        onOpened: { recHover.close(); page.openShow(modelData) }
                        onRested: function(on) { on ? recHover.enter(recTile, modelData) : recHover.leave(recTile) }
                    }
                }
            }
            Label {
                visible: recBox.shown.length === 0
                text: page.recScope === "InLibrary" ? words.t("home-recommended-none-library")
                    : words.t("home-recommended-none-list")
                role: "ui"
                color: theme.textDim
            }
        }
        Item { width: 1; height: recBox.visible ? theme.space(4) : 0 }

        Shelf {
            objectName: "home-catch-up"
            width: parent.width
            heading: words.t("home-catch-up")
            fact: page.catchUpList.length ? words.t("home-catch-up-fact", { count: page.catchUpList.length }) : ""
            list: page.catchUpList
            maxRows: 2
            onMore: frame.go("library", { arrange: "WatchState", box: "Watching" })
        }
        Item { width: 1; height: page.catchUpList.length ? theme.space(4) : 0 }

        Shelf {
            objectName: "home-fresh"
            width: parent.width
            heading: words.t("home-fresh")
            fact: words.t("home-fresh-fact", { count: page.freshList.length })
            list: page.freshList
            maxRows: 1
            onMore: frame.go("library", { arrange: "WatchState", box: "Not started" }) // i18n: none
        }
        Item { width: 1; height: page.freshList.length ? theme.space(4) : 0 }

        // ---- On hold: off the top of the page (#1464), still a row away
        Shelf {
            objectName: "home-on-hold"
            width: parent.width
            heading: words.t("home-on-hold")
            fact: words.t("home-on-hold-fact", { count: page.holdList.length })
            list: page.holdList
            maxRows: 1
            onMore: frame.go("library", { arrange: "WatchState", box: "On hold" }) // i18n: none
        }
        Item { width: 1; height: page.holdList.length ? theme.space(4) : 0 }

        // ---- Genres: a way into the library by kind
        Column {
            objectName: "home-genres"
            visible: page.genreList.length > 0
            width: parent.width
            spacing: theme.space(3)
            Heading {
                text: words.t("home-genres")
                action: Button {
                    msg: "home-open-library"
                    icon: "layers"
                    small: true
                    flat: true
                    onClicked: frame.go("library")
                }
            }
            Flow {
                width: parent.width
                spacing: theme.space(2)
                Repeater {
                    model: page.genreList
                    Rectangle {
                        id: genre
                        required property var modelData
                        width: genreRow.implicitWidth + theme.space(6)
                        height: theme.space(10)
                        radius: height / 2
                        color: gm.containsMouse ? theme.raised : theme.tile
                        border.color: gm.containsMouse ? theme.lineStrong : "transparent"
                        border.width: 1
                        scale: theme.reducedMotion ? 1 : (gm.pressed ? 0.97 : 1)
                        activeFocusOnTab: true
                        function open() { frame.go("library", { genre: genre.modelData.name }, genre.modelData.name) }
                        Keys.onReturnPressed: open()
                        Keys.onSpacePressed: open()
                        Rectangle { anchors.fill: parent; anchors.margins: -2; radius: height / 2; visible: genre.activeFocus; color: "transparent"; border.width: 2; border.color: theme.focusRing }
                        Row {
                            id: genreRow
                            anchors.centerIn: parent
                            spacing: theme.space(2)
                            Label { text: genre.modelData.name; role: "ui"; anchors.verticalCenter: parent.verticalCenter }
                            Label { text: genre.modelData.count; role: "ledger"; color: theme.textFaint; anchors.verticalCenter: parent.verticalCenter }
                        }
                        MouseArea {
                            Accessible.role: Accessible.Button
                            Accessible.onPressAction: genre.open()
                            Accessible.name: theme.nameNear(parent)
                            id: gm
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: genre.open()
                        }
                    }
                }
            }
        }

        // ---- Dropped: last, since nothing here is to be watched
        Item { width: 1; height: page.droppedList.length && page.genreList.length ? theme.space(4) : 0 }
        Shelf {
            objectName: "home-dropped"
            width: parent.width
            heading: words.t("home-dropped")
            fact: words.t("home-dropped-fact", { count: page.droppedFiles })
            list: page.droppedList
            maxRows: 1
            onMore: frame.go("library", { arrange: "WatchState", box: "Dropped" })
        }
    }

    // ---- No library yet: the same empty state as Library, and no rows (#1006)
    // Centred in the room above the tracker nudge, so the two never meet; on a window too
    // short for both, the nudge steps aside (#1107)
    LibraryEmpty {
        id: homeEmpty
        objectName: "home-empty"
        visible: page.libraryEmpty
        anchors.horizontalCenter: parent.horizontalCenter
        readonly property real room: nudge.shown ? nudge.y - theme.space(4) : page.height
        y: Math.max(0, (room - height) / 2)
    }
    EmptyState {
        objectName: "home-failed"
        anchors.centerIn: parent
        visible: page.loadFailed && !page.libraryEmpty
        icon: "circle-alert"
        title: words.t("home-failed-title")
        body: words.t("home-failed-body")
        Button { msg: "action-try-again"; icon: "refresh-cw"; primary: true; onClicked: page.reload() }
    }
    // A new install has no library to hold the tracker nudge, so the empty Home carries it
    TrackerNudge {
        id: nudge
        objectName: "home-tracker-nudge"
        readonly property bool shown: page.libraryEmpty && wanted && homeEmpty.height + theme.space(8) <= y
        visible: shown
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        // Over the update island when the two share the page's right side, never under it (#1093)
        readonly property bool underIsland: frame.islandReach > 0 && x + width > page.width - frame.islandReach
        anchors.bottomMargin: underIsland ? Math.max(theme.space(10), frame.islandRise + theme.space(4)) : theme.space(10)
        width: Math.min(parent.width - theme.space(16), theme.space(220))
        height: implicitHeight
    }
}
