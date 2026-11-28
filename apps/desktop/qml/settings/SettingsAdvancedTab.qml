// Advanced: Usage log, Tailnet (Phone app host service, addresses, people, and trusted
// devices), Watch together relay, Developer options, and Setup & Help.
import QtQuick
import QtQuick.Dialogs
import QtCore
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-advanced-title")

    // ---- Usage log (#1532)
    property var usage: ({ text: "", days: 0, events: 0, enabled: true })
    property bool confirmingUsageClear: false
    property bool sendingUsage: false
    property real usageToken: -1
    property string lastUsageFile: ""
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
    function sendUsage() {
        var path = tab.usageFile()
        var shown = usagePreview.text
        if (!tab.saveUsage(path, shown)) return
        var t = Door.callAsync("SendUsage", { text: shown })
        if (t < 0) { frame.toast(words.t("settings-usage-send-not-started", { path: path })); return }
        tab.usageToken = t
        tab.sendingUsage = true
        tab.lastUsageFile = path
    }
    function clearUsage() {
        var r = Door.call("ClearUsage", {})
        if (r.error) { frame.fail(r.error, tab.clearUsage); return }
        tab.loadUsage()
        frame.toast(words.t("settings-usage-cleared"))
    }
    function path(url) { return Fmt.localPath(String(url)) }

    // ---- Tailnet / Phone app details
    property var host: ({ enabled: false, urls: [], message: "" })
    readonly property string enableCommand: "systemctl --user enable --now anibeam-host" // i18n: none
    property string service: ""
    property var devices: []
    property var people: []
    function reloadHost() { var r = Door.getHost(); if (r.error) frame.fail(r.error, tab.reloadHost); else host = r.reply.status }
    function reloadDevices() {
        var r = Door.call("ListTailnetDevices", {})
        if (r.error) return
        devices = r.reply.devices
        people = r.reply.people || []
    }
    function setTrusted(device, on) {
        var r = Door.call("SetTailnetDeviceTrust", { device: device, trusted: on })
        if (r.error) frame.fail(r.error, function() { tab.setTrusted(device, on) })
        reloadDevices()
    }
    function deviceLine(d) {
        var names = { linux: "Linux", windows: "Windows", android: "Android", ios: "iOS", macos: "macOS", freebsd: "FreeBSD", openbsd: "OpenBSD" } // i18n: none
        var os = names[d.os.toLowerCase()] || d.os
        var parts = []
        if (os !== "") parts.push(os)
        parts.push(d.online ? words.t("settings-phone-online") : words.t("settings-phone-offline"))
        if (d.person) parts.push(d.person)
        if (d.nobody) parts.push(words.t("settings-phone-nobody"))
        else if (!d.trusted) parts.push(words.t("settings-phone-untrusted"))
        return parts.join(" · ")
    }
    function personLine(p) {
        var parts = []
        if (p.login && p.login !== p.name) parts.push(p.login)
        parts.push(words.t(p.owner ? "settings-phone-person-own-devices" : "settings-phone-person-devices", { count: p.devices }))
        return parts.join(" · ")
    }
    function readPhone() { reloadHost(); reloadDevices(); service = Shell.hostService() }

    Component.onCompleted: { loadUsage(); Qt.callLater(tab.readPhone) }
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
        function onTailnetDevicesChanged(devices, people) { tab.devices = devices; tab.people = people }
        function onHostChanged(status) { tab.host = status }
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
        objectName: "settings-tailnet"
        title: words.t("settings-advanced-tailnet")
        SettingRow {
            objectName: "settings-host-service"
            label: words.t("settings-phone-host-service")
            line: tab.service === "" ? words.t("settings-phone-not-running") : words.t("settings-phone-running")
            Button {
                visible: tab.service === ""
                msg: "settings-phone-copy-command"; icon: "copy"; flat: true; small: true
                onClicked: frame.copyText(tab.enableCommand, words.t("settings-phone-command-copied"))
            }
        }
        Repeater {
            model: tab.host.enabled ? tab.host.urls : []
            SettingRow {
                required property var modelData
                label: modelData
                redactLabel: true
                Button { msg: "settings-phone-copy"; icon: "copy"; flat: true; small: true; onClicked: frame.copyText(modelData, words.t("settings-phone-address-copied")) }
            }
        }
    }
    Panel {
        objectName: "settings-tailnet-people"
        visible: tab.people.length > 0
        title: words.t("settings-phone-people")
        Repeater {
            model: tab.people
            SettingRow {
                required property var modelData
                label: modelData.name
                line: tab.personLine(modelData)
                redactLabel: true
                redactLine: true
                Chip {
                    visible: modelData.owner
                    text: words.t("settings-phone-own-profile")
                    mono: false
                    small: true
                    color: theme.raised
                }
            }
        }
    }
    Panel {
        objectName: "settings-tailnet-devices"
        visible: tab.devices.length > 0
        title: words.t("settings-phone-devices")
        Repeater {
            model: tab.devices
            SettingRow {
                required property var modelData
                label: modelData.name
                line: tab.deviceLine(modelData)
                redactLabel: true
                redactLine: true
                Switch {
                    checked: modelData.trusted
                    onToggled: function(on) { tab.setTrusted(modelData.id, on) }
                }
            }
        }
    }
    Panel {
        title: words.t("settings-playback-together")
        SettingRow { label: words.t("settings-playback-room-relay")
            PrivacyBlur {
                id: roomRelayBlur
                width: roomRelayField.width; height: roomRelayField.height
                Field { id: roomRelayField; objectName: "settings-room-relay"; text: roomRelayBlur.blurred ? roomRelayBlur.mask(Door.preferences.room_relay || "") : (Door.preferences.room_relay || ""); mono: true; width: theme.space(64)
                    accessibleName: words.t("settings-playback-room-relay")
                    onEdited: function(t) { if (t.trim() !== (Door.preferences.room_relay || "")) frame.room.setRelay(t) } }
            } }
    }
    Panel {
        visible: !Dev.forced
        title: words.t("settings-about-developer")
        SettingRow { label: words.t("settings-about-developer")
            Switch { objectName: "data-developer"; checked: frame.devKept.enabled; onToggled: function(on) { frame.devKept.enabled = on; frame.devApply(); if (on) frame.go("settings", { tab: "developer" }) } } }
    }
    Panel {
        objectName: "help"
        title: words.t("settings-about-help")
        SettingRow { label: words.t("settings-about-setup")
            Button { objectName: "about-run-setup"; msg: "settings-about-run-setup"; icon: "rotate-ccw"; flat: true; onClicked: frame.openOnboarding(0) } }
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
}
