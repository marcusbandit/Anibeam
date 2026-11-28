// Connections: Discord (Rich Presence and blocked users) and the Phone app hot switches
// (Share over Tailscale and Stream episodes). Tailscale addresses, host service, people
// and trusted devices live in Advanced.
import QtQuick
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-connections-title")

    // ---- Discord
    property bool discordAvailable: false
    property bool presenceFailed: false
    property var discordProblem: null
    property bool discordConnected: false
    property var discordBlocked: []
    function setDiscord(on) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.discord_presence = on
        var r = Door.setPreferences(p)
        if (r.error) frame.fail(r.error, function() { tab.setDiscord(on) })
    }
    function readPresence() {
        var presence = Door.call("GetPresence", {})
        presenceFailed = !!presence.error
        if (!presence.error) { discordAvailable = presence.reply.available; discordConnected = presence.reply.connected; discordProblem = presence.reply.problem || null }
    }
    readonly property string discordState: !Door.preferences.discord_presence ? ""
        : discordProblem === "NoDiscord" ? words.t("settings-playback-discord-no-discord")
        : discordProblem && discordProblem.Refused ? words.t("settings-playback-discord-refused", { reason: discordProblem.Refused.message })
        : discordConnected ? words.t("settings-playback-discord-connected") : words.t("settings-playback-discord-not-connected")
    function readBlocked() {
        var r = Door.call("DiscordBlocked", {})
        if (!r.error) discordBlocked = r.reply.people || []
    }
    function clearBlocked() {
        var r = Door.call("DiscordUnblock", { user_id: null })
        if (r.error) { frame.fail(r.error); return }
        discordBlocked = r.reply.people || []
    }

    // ---- Phone app hot switches
    property var host: ({ enabled: false, urls: [], message: "" })
    property string service: ""
    readonly property bool sharing: !!Door.preferences.share_over_tailscale
    function reloadHost() { var r = Door.getHost(); if (r.error) frame.fail(r.error, tab.reloadHost); else host = r.reply.status }
    function setSharing(on) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.share_over_tailscale = on
        var r = Door.setPreferences(p)
        if (r.error) frame.fail(r.error, function() { tab.setSharing(on) })
    }
    function setStreaming(on) {
        var p = JSON.parse(JSON.stringify(Door.preferences))
        p.stream_over_tailscale = on
        var r = Door.setPreferences(p)
        if (r.error) frame.toast(r.error.message)
    }

    Component.onCompleted: { readPresence(); readBlocked(); Qt.callLater(function() { reloadHost(); service = Shell.hostService() }) }
    Connections { target: Door; function onPresenceChanged() { tab.readPresence() } }
    Connections { target: Door; function onHostChanged(status) { tab.host = status } }

    Panel {
        title: "Discord" // i18n: none
        SettingRow { label: words.t("settings-playback-discord")
            status: tab.presenceFailed ? words.t("settings-playback-discord-check-failed")
                : !tab.discordAvailable ? words.t("settings-playback-discord-unavailable")
                : tab.discordState
            Row { spacing: theme.space(2)
                Button { visible: tab.presenceFailed; msg: "action-try-again"; icon: "refresh-cw"; flat: true; small: true; onClicked: tab.readPresence() }
                Switch { objectName: "settings-discord-switch"; enabled: tab.discordAvailable; opacity: enabled ? 1 : theme.disabledOpacity; checked: !!Door.preferences.discord_presence; onToggled: function(on) { tab.setDiscord(on) } } } }
        SettingRow { label: words.t("settings-playback-discord-blocked")
            visible: tab.discordAvailable
            status: tab.discordBlocked.map(function(p) { return p.name }).join(", ")
            redactStatus: tab.discordBlocked.length > 0
            Button { objectName: "settings-discord-unblock"; msg: "action-clear"; flat: true; small: true
                enabled: tab.discordBlocked.length > 0; opacity: enabled ? 1 : theme.disabledOpacity
                onClicked: tab.clearBlocked() } }
    }
    Panel {
        objectName: "settings-share"
        title: words.t("settings-phone-title")
        SettingRow {
            label: words.t("settings-phone-share")
            line: tab.host.message
            Switch {
                objectName: "settings-share-switch"
                checked: tab.sharing || tab.service !== ""
                enabled: tab.service === ""
                opacity: enabled ? 1 : theme.disabledOpacity
                onToggled: function(on) { tab.setSharing(on) }
            }
        }
        SettingRow {
            label: words.t("settings-phone-stream")
            Switch {
                objectName: "settings-stream-switch"
                checked: Door.preferences.stream_over_tailscale !== false
                onToggled: function(on) { tab.setStreaming(on) }
            }
        }
        SettingRow {
            icon: "sliders-horizontal"
            label: words.t("settings-connections-tailnet-details")
            clickable: true
            trailingIcon: "chevron-right"
            onClicked: page.open("advanced", words.t("settings-phone-host-service"))
        }
    }
}
