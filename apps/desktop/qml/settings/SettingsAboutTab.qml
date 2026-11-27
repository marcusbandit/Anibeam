// About: the version with What's new first (an update on offer, its progress or its failure
// under it), setup again and the update channel with a check. Then storage and backup: Back
// up the library (one file, with private data or without), Restore from a backup with what
// the last restore did, the usage log (#1532: keep it or not, how much it holds, save it,
// look at it and send it once, clear it), the image cache with its size and a clear, and
// where Anibeam keeps its files; export, import and the clear are jobs whose results come back as
// exportFinished, importFinished and imagesCleared. Then developer options (the switch,
// hidden only when ANIBEAM_DEV=1 forces them on), then Help: how the folders should look and
// where to report a problem. Last the credits, IMDb's among them, which its terms ask for
// wherever its data is used.
import QtQuick
import QtQuick.Dialogs
import QtCore
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-about-title")
    readonly property var upd: frame.updatePanel
    // An update waits for this track, put away or not; About keeps the way to take it until
    // it is installed or the track changes
    readonly property bool offered: upd.pending && upd.phase === "offer"
    readonly property string offerLine: upd.phase === "failed" ? words.t("settings-about-update-failed")
        : upd.pending && (upd.phase === "downloading" || upd.phase === "ready") ? words.t("settings-about-update-progress", { headline: upd.headline, detail: upd.detail })
        : !offered ? ""
        : upd.how === "install" ? words.t("settings-about-update-ready", { version: upd.version })
        : upd.how === "command" ? words.t("settings-about-update-command", { version: upd.version, command: upd.command })
        : upd.how === "source" ? words.t("settings-about-update-source", { version: upd.version })
        : words.t("settings-about-update-elsewhere", { version: upd.version })
    readonly property var about: Door.about || ({})
    Component.onCompleted: { loadStorage(); loadUsage() }

    // ---- Usage log (#1532): counted in the core, sent only from the preview's Send
    property var usage: ({ text: "", days: 0, events: 0, enabled: true })
    property bool confirmingUsageClear: false
    property bool sendingUsage: false
    property real usageToken: -1
    readonly property string downloadsDir: Fmt.localPath(String(StandardPaths.writableLocation(StandardPaths.DownloadLocation)))
    function usageFile() { return tab.downloadsDir + "/anibeam-usage-" + new Date().toISOString().slice(0, 10) + ".json" } // i18n: none
    function loadUsage() { var r = Door.call("GetUsage", {}); if (!r.error) usage = r.reply.usage; return r }
    function setUsageLog(on) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.usage_log = on
        var r = Door.setPreferences(p)
        if (r && r.error) frame.fail(r.error, function() { tab.setUsageLog(on) })
        tab.loadUsage()
    }
    function saveUsage(path, text) {
        var r = Door.call("ExportUsage", { path: path, text: text === undefined ? null : text })
        if (r.error) { frame.fail(r.error, function() { tab.saveUsage(path, text) }); return false }
        tab.loadUsage()
        return true
    }
    function previewUsage() {
        var r = tab.loadUsage()
        if (r.error) { frame.fail(r.error, tab.previewUsage); return }
        usagePreview.openWith(tab.usage.text)
    }
    // Send: the file is saved first, then the log goes out once, off the Qt thread
    function sendUsage() {
        var path = tab.usageFile()
        // What goes out is the text the preview showed, not the log a moment later
        var shown = usagePreview.text
        if (!tab.saveUsage(path, shown)) return
        var t = Door.callAsync("SendUsage", { text: shown })
        if (t < 0) { frame.toast(words.t("settings-usage-send-not-started", { path: path })); return }
        tab.usageToken = t
        tab.sendingUsage = true
        tab.lastUsageFile = path
    }
    property string lastUsageFile: ""
    function clearUsage() {
        var r = Door.call("ClearUsage", {})
        if (r.error) { frame.fail(r.error, tab.clearUsage); return }
        tab.loadUsage()
        frame.toast(words.t("settings-usage-cleared"))
    }
    Connections {
        target: Door
        function onCallDone(token, reply) {
            if (token !== tab.usageToken) return
            tab.usageToken = -1
            tab.sendingUsage = false
            usagePreview.close()
            tab.loadUsage()
            if (reply.error) frame.toast(words.t("settings-usage-send-failed", { error: reply.error.plain || reply.error.message || "" }))
            else frame.toast(words.t("settings-usage-sent"), 0, "", null, words.t("settings-usage-saved", { path: tab.lastUsageFile }))
        }
    }

    // ---- Storage and backup
    property bool includePrivate: false
    property string lastExport: ""
    property var importSummary: null
    property var storage: ({ image_count: 0, image_bytes: 0 })
    property bool confirmingClear: false
    // A cache that could not be measured says so, rather than reading as empty
    property bool storageFailed: false
    function loadStorage() { var r = Door.getStorage(); storageFailed = !!r.error; if (!r.error) storage = r.reply }
    function clearImages() { var r = Door.clearImages(); if (r.error) frame.fail(r.error, tab.clearImages) }
    function path(url) { return Fmt.localPath(String(url)) }
    function dir(p) { var i = Math.max(String(p || "").lastIndexOf("/"), String(p || "").lastIndexOf("\\")); return i > 0 ? p.slice(0, i) : p }
    Connections {
        target: Door
        function onExportFinished(p) { tab.lastExport = p; frame.toast(words.t("settings-data-backed-up", { path: p })) }
        function onImportFinished(summary) { tab.importSummary = summary; frame.toast(words.t("settings-data-restore-finished")) }
        function onImagesCleared(removed) { tab.loadStorage(); frame.toast(words.t("settings-data-images-removed", { count: removed })) }
        function onJobFailed(job, kind, error) {
            if (kind === "Export") frame.fail(error, exportDialog.open)
            else if (kind === "Import") frame.fail(error, importDialog.open)
            else if (kind === "ClearImages") frame.fail(error, tab.clearImages)
        }
    }
    function summaryLine(s) {
        if (!s) return ""
        var parts = [words.t("settings-data-summary-folders", { count: s.sources_added }),
                     words.t("settings-data-summary-series", { count: s.series_created }),
                     words.t("settings-data-summary-matches", { count: s.matches_applied }),
                     words.t("settings-data-summary-views", { views: s.views_merged, episodes: s.completed_merged }),
                     words.t("settings-data-summary-resume-points", { count: s.resume_points_merged }),
                     words.t("settings-data-summary-accounts", { count: s.accounts_imported })]
        var out = [words.t("settings-data-summary", { parts: parts.join(", ") })]
        if (s.fixes_applied > 0) out.push(words.t("settings-data-summary-fixes", { count: s.fixes_applied }))
        if (s.sources_unavailable > 0) out.push(words.t("settings-data-summary-unavailable", { count: s.sources_unavailable }))
        if (s.fields_ignored && s.fields_ignored.length) out.push(words.t("settings-data-summary-left-out", { fields: s.fields_ignored.join(", ") }))
        return out.join(" ")
    }


    Panel {
        title: words.t("settings-about-version")
        SettingRow { label: words.t("settings-about-build", { build: Shell.build })
            status: tab.offerLine
            Row { spacing: theme.space(2)
                Button { objectName: "data-whats-new"; msg: "settings-about-whats-new"; icon: "sparkles"; flat: true; onClicked: frame.openWhatsNew() }
                Button { objectName: "about-update-install"; visible: tab.offered && tab.upd.how === "install"; msg: "settings-about-install"; msgArgs: ({ version: tab.upd.version }); icon: "download"; primary: true; onClicked: tab.upd.install() }
                Button { objectName: "about-update-copy"; visible: tab.offered && tab.upd.how === "command"; text: tab.upd.copyText; icon: tab.upd.copied ? "check" : "copy"; primary: true; onClicked: tab.upd.copyCommand() }
                Button { objectName: "about-update-restart"; visible: tab.upd.phase === "ready"; text: tab.upd.restartText; icon: "refresh-cw"; primary: true; onClicked: tab.upd.restart() }
                Button { objectName: "about-update-retry"; visible: tab.upd.phase === "failed"; text: tab.upd.retryText; icon: "rotate-cw"; primary: true; onClicked: tab.upd.install() }
                Button { objectName: "about-update-cancel"; visible: tab.upd.phase === "downloading"; text: tab.upd.cancelText; icon: "x"; flat: true; onClicked: tab.upd.cancel() }
                Button { objectName: "about-update-page"; visible: (tab.upd.phase === "failed" || (tab.offered && (tab.upd.how === "page" || tab.upd.how === "source"))) && tab.upd.url !== ""
                         text: tab.upd.pageText; icon: "external-link"; primary: tab.upd.how === "page" && tab.upd.phase !== "failed"; flat: !primary; onClicked: frame.openLink(tab.upd.url) } } }
        SettingRow { label: words.t("settings-about-setup")
            Button { objectName: "about-run-setup"; msg: "settings-about-run-setup"; icon: "rotate-ccw"; flat: true; onClicked: frame.openOnboarding(0) } }
        SettingRow { label: words.t("settings-about-updates")
            Row { spacing: theme.space(2)
                Seg { objectName: "data-update-track"; options: [{ text: words.t("settings-about-stable"), icon: "badge-check" }, { text: words.t("settings-about-nightly"), icon: "moon" }]
                      index: tab.upd.track === "nightly" ? 1 : 0
                      onPicked: function(i) { tab.upd.setTrack(i === 1 ? "nightly" : "stable") } }
                Button { objectName: "data-check-updates"; msg: "settings-about-check-now"; icon: "refresh-cw"; flat: true; onClicked: tab.upd.checkNow() } } }
    }
    Panel {
        title: words.t("settings-data-backup")
        SettingRow { label: words.t("settings-data-back-up-library")
            line: tab.lastExport !== "" ? words.t("settings-data-last-written", { path: tab.lastExport }) : ""
            redactLine: tab.lastExport !== ""
            Row { spacing: theme.space(2)
                Button { visible: tab.lastExport !== ""; msg: "settings-data-open-folder"; icon: "folder-open"; flat: true; onClicked: frame.openLink(Fmt.fileUrl(tab.dir(tab.lastExport))) }
                Button { objectName: "data-export"; msg: "settings-data-back-up"; icon: "file-up"; primary: true; onClicked: exportDialog.open() } } }
        SettingRow { label: words.t("settings-data-private")
            Switch { objectName: "data-private"; checked: tab.includePrivate; onToggled: function(on) { tab.includePrivate = on } } }
        SettingRow { label: words.t("settings-data-restore-from")
            status: tab.summaryLine(tab.importSummary)
            Button { objectName: "data-import"; msg: "settings-data-restore"; icon: "file-down"; flat: true; onClicked: importDialog.open() } }
    }
    Panel {
        objectName: "usage-log"
        title: words.t("settings-about-usage")
        SettingRow { label: words.t("settings-usage-keep")
            Switch { objectName: "usage-keep"; checked: Door.preferences.usage_log !== false; onToggled: function(on) { tab.setUsageLog(on) } } }
        SettingRow {
            label: tab.usage.events > 0 ? words.t("settings-usage-summary", { days: tab.usage.days, events: tab.usage.events }) : words.t("settings-usage-empty")
            Item {
                width: tab.confirmingUsageClear ? usageConfirm.implicitWidth : usageButtons.width
                height: tab.confirmingUsageClear ? usageConfirm.implicitHeight : usageButtons.height
                Row {
                    id: usageButtons
                    visible: !tab.confirmingUsageClear
                    spacing: theme.space(2)
                    Button { objectName: "usage-clear"; msg: "settings-usage-clear"; icon: "trash-2"; flat: true; enabled: tab.usage.events > 0; onClicked: tab.confirmingUsageClear = true }
                    Button { objectName: "usage-download"; msg: "settings-usage-download"; icon: "file-down"; flat: true; onClicked: usageDialog.open() }
                    Button { objectName: "usage-send"; msg: "settings-usage-send"; icon: "send"; primary: true; enabled: !tab.sendingUsage; onClicked: tab.previewUsage() }
                }
                InlineConfirm {
                    id: usageConfirm
                    visible: tab.confirmingUsageClear
                    question: words.t("settings-usage-clear-confirm")
                    confirmText: words.t("settings-usage-clear")
                    onAccepted: { tab.confirmingUsageClear = false; tab.clearUsage() }
                    onKept: tab.confirmingUsageClear = false
                }
            }
        }
    }
    Panel {
        objectName: "data-images"
        title: words.t("settings-data-image-cache")
        SettingRow {
            label: tab.storageFailed ? words.t("settings-data-cache-unmeasured") : tab.storage.image_count ? words.t("settings-data-cache-size", { size: words.bytes(tab.storage.image_bytes || 0), count: tab.storage.image_count }) : words.t("settings-data-cache-empty")
            Item {
                width: tab.confirmingClear ? confirm.implicitWidth : clearBtn.width
                height: tab.confirmingClear ? confirm.implicitHeight : clearBtn.height
                Button { id: clearBtn; objectName: "data-clear-images"; visible: !tab.confirmingClear; text: tab.storageFailed ? words.t("action-try-again") : words.t("action-clear"); icon: tab.storageFailed ? "refresh-cw" : "trash-2"; flat: true; enabled: tab.storageFailed || tab.storage.image_count > 0; onClicked: tab.storageFailed ? tab.loadStorage() : tab.confirmingClear = true }
                InlineConfirm {
                    id: confirm
                    visible: tab.confirmingClear
                    question: words.t("settings-data-remove-images", { count: tab.storage.image_count })
                    confirmText: words.t("action-clear")
                    onAccepted: { tab.confirmingClear = false; tab.clearImages() }
                    onKept: tab.confirmingClear = false
                }
            }
        }
    }
    Panel {
        title: words.t("settings-data-files")
        Repeater {
            model: [["settings-data-files-database", "db_path", true], ["settings-data-files-data", "data_dir", false], ["settings-data-files-settings", "config_dir", false], ["settings-data-files-cache", "cache_dir", false]]
            SettingRow {
                required property var modelData
                label: words.t(modelData[0])
                status: tab.about[modelData[1]] || ""
                redactStatus: true
                Button { msg: "action-open"; icon: "folder-open"; flat: true; small: true; onClicked: frame.openLink(Fmt.fileUrl(modelData[2] ? tab.dir(tab.about[modelData[1]]) : tab.about[modelData[1]])) }
            }
        }
    }
    Panel {
        visible: !Dev.forced
        title: words.t("settings-about-developer")
        // Developer options (#287): off by default in every build; ANIBEAM_DEV=1 forces them
        // on and hides this switch
        SettingRow { label: words.t("settings-about-developer")
            Switch { objectName: "data-developer"; checked: frame.devKept.enabled; onToggled: function(on) { frame.devKept.enabled = on; frame.devApply(); if (on) frame.go("settings", { tab: "developer" }) } } }
    }
    // Help: how the folders should look, and where to report what is wrong
    Panel {
        objectName: "help"
        title: words.t("settings-about-help")
        SettingRow {
            label: words.t("settings-help-folders")
            line: words.t("settings-help-folders-example")
        }
        SettingRow {
            label: words.t("settings-help-more")
            Button { objectName: "help-report"; msg: "settings-help-report"; icon: "external-link"; flat: true
                     onClicked: frame.openLink("https://github.com/marcusbandit/Anibeam/issues/new?template=2-bug-report.yml&version=" + encodeURIComponent(Shell.release)) }
        }
    }
    Panel {
        title: words.t("settings-about-credits")
        SettingRow { label: words.t("settings-about-ratings"); line: words.t("settings-about-ratings-helper") }
        SettingRow { label: words.t("settings-about-series"); line: words.t("settings-about-series-helper") }
    }

    FileDialog {
        id: exportDialog
        title: words.t("settings-data-back-up-library")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "json"
        currentFile: Fmt.fileUrl((tab.about.data_dir ? tab.dir(tab.about.data_dir) : "") + "/anibeam-export.json")
        nameFilters: [words.t("settings-data-backup-filter")]
        onAccepted: { var r = Door.exportLibrary(tab.path(selectedFile), tab.includePrivate); r.error ? frame.fail(r.error, exportDialog.open) : frame.toast(words.t("settings-data-backing-up")) }
    }
    FileDialog {
        id: usageDialog
        title: words.t("settings-usage-download")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "json"
        currentFile: Fmt.fileUrl(tab.usageFile())
        nameFilters: [words.t("settings-usage-filter")]
        onAccepted: { var path = tab.path(selectedFile); if (tab.saveUsage(path)) frame.toast(words.t("settings-usage-saved", { path: path })) }
    }
    UsagePreview {
        id: usagePreview
        parent: frame.overlay
        anchors.centerIn: parent
        sending: tab.sendingUsage
        onSend: tab.sendUsage()
    }
    Blocker {
        parent: frame.overlay
        anchors.fill: parent
        z: -1
        visible: usagePreview.visible
        dismissOnPress: true
        onDismissed: usagePreview.close()
    }
    FileDialog {
        id: importDialog
        title: words.t("settings-data-restore-from")
        nameFilters: [words.t("settings-data-backup-filter"), words.t("settings-data-all-files-filter")]
        onAccepted: { var r = Door.importLibrary(tab.path(selectedFile)); r.error ? frame.fail(r.error, importDialog.open) : frame.toast(words.t("settings-data-restoring")) }
    }
}
