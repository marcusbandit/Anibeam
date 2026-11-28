// About: streamlined app information — Version & Updates, a clean clickable list
// (What's new, Help contribute, Help translate, Open source licenses, Privacy policy),
// quiet attribution credits, and 3 bottom brand icon links (Website, Discord, GitHub).
import QtQuick
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-about-title")
    readonly property var upd: frame.updatePanel
    readonly property bool offered: upd.pending && upd.phase === "offer"
    readonly property string offerLine: upd.phase === "failed" ? words.t("settings-about-update-failed")
        : upd.pending && (upd.phase === "downloading" || upd.phase === "ready") ? words.t("settings-about-update-progress", { headline: upd.headline, detail: upd.detail })
        : !offered ? ""
        : upd.how === "install" ? words.t("settings-about-update-ready", { version: upd.version })
        : upd.how === "command" ? words.t("settings-about-update-command", { version: upd.version, command: upd.command })
        : upd.how === "source" ? words.t("settings-about-update-source", { version: upd.version })
        : words.t("settings-about-update-elsewhere", { version: upd.version })

    readonly property string websiteUrl: "https://anibeam.app" // i18n: none
    readonly property string discordUrl: "" // i18n: none
    readonly property string githubUrl: "https://github.com/marcusbandit/Anibeam" // i18n: none

    Panel {
        title: words.t("settings-about-version")
        SettingRow { label: words.t("settings-about-build", { build: Shell.build })
            status: tab.offerLine
            Row { spacing: theme.space(2)
                Button { objectName: "about-update-install"; visible: tab.offered && tab.upd.how === "install"; msg: "settings-about-install"; msgArgs: ({ version: tab.upd.version }); icon: "download"; primary: true; onClicked: tab.upd.install() }
                Button { objectName: "about-update-copy"; visible: tab.offered && tab.upd.how === "command"; text: tab.upd.copyText; icon: tab.upd.copied ? "check" : "copy"; primary: true; onClicked: tab.upd.copyCommand() }
                Button { objectName: "about-update-restart"; visible: tab.upd.phase === "ready"; text: tab.upd.restartText; icon: "refresh-cw"; primary: true; onClicked: tab.upd.restart() }
                Button { objectName: "about-update-retry"; visible: tab.upd.phase === "failed"; text: tab.upd.retryText; icon: "rotate-cw"; primary: true; onClicked: tab.upd.install() }
                Button { objectName: "about-update-cancel"; visible: tab.upd.phase === "downloading"; text: tab.upd.cancelText; icon: "x"; flat: true; onClicked: tab.upd.cancel() }
                Button { objectName: "about-update-page"; visible: (tab.upd.phase === "failed" || (tab.offered && (tab.upd.how === "page" || tab.upd.how === "source"))) && tab.upd.url !== ""
                         text: tab.upd.pageText; icon: "external-link"; primary: tab.upd.how === "page" && tab.upd.phase !== "failed"; flat: !primary; onClicked: frame.openLink(tab.upd.url) } } }
        SettingRow { label: words.t("settings-about-updates")
            Row { spacing: theme.space(2)
                Seg { objectName: "data-update-track"; options: [{ text: words.t("settings-about-stable"), icon: "badge-check" }, { text: words.t("settings-about-nightly"), icon: "moon" }]
                      index: tab.upd.track === "nightly" ? 1 : 0
                      onPicked: function(i) { tab.upd.setTrack(i === 1 ? "nightly" : "stable") } }
                Button { objectName: "data-check-updates"; msg: "settings-about-check-now"; icon: "refresh-cw"; flat: true; onClicked: tab.upd.checkNow() } } }
    }
    Panel {
        SettingRow {
            objectName: "data-whats-new"
            icon: "sparkles"
            label: words.t("settings-about-whats-new")
            clickable: true
            trailingIcon: "chevron-right"
            onClicked: frame.openWhatsNew()
        }
        SettingRow {
            objectName: "about-contribute"
            icon: "git-branch"
            label: words.t("settings-about-help-contribute")
            clickable: true
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.githubUrl + "/blob/main/CONTRIBUTING.md")
        }
        SettingRow {
            objectName: "about-translate"
            icon: "languages"
            label: words.t("settings-about-help-translate")
            clickable: true
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.githubUrl + "/tree/main/i18n")
        }
        SettingRow {
            objectName: "about-licenses"
            icon: "book-open"
            label: words.t("settings-about-licenses")
            clickable: true
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.githubUrl + "/blob/main/LICENSE")
        }
        SettingRow {
            objectName: "about-privacy-policy"
            icon: "lock"
            label: words.t("settings-about-privacy-policy")
            clickable: true
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.websiteUrl + "/privacy/")
        }
    }
    Panel {
        title: words.t("settings-about-credits")
        SettingRow { label: words.t("settings-about-ratings"); line: words.t("settings-about-ratings-helper") }
        SettingRow { label: words.t("settings-about-series"); line: words.t("settings-about-series-helper") }
    }
    Panel {
        bare: true
        stretch: Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: theme.space(4)
            Repeater {
                model: [
                    { id: "website", icon: "globe", tip: words.t("settings-about-link-website"), url: tab.websiteUrl },
                    { id: "discord", icon: "discord", tip: words.t("settings-about-link-discord"), url: tab.discordUrl },
                    { id: "github", icon: "github", tip: words.t("settings-about-link-github"), url: tab.githubUrl }
                ]
                Rectangle {
                    id: socialBtn
                    required property var modelData
                    objectName: "about-link-" + modelData.id
                    width: theme.space(12)
                    height: theme.space(12)
                    radius: theme.radiusTile
                    color: socialMouse.containsMouse && modelData.url !== "" ? theme.raised : theme.tile
                    opacity: modelData.url !== "" ? 1 : theme.disabledOpacity
                    Icon {
                        anchors.centerIn: parent
                        glyph: socialBtn.modelData.icon
                        size: theme.space(5.5)
                        color: socialMouse.containsMouse && socialBtn.modelData.url !== "" ? theme.accent : theme.text
                    }
                    MouseArea {
                        id: socialMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: socialBtn.modelData.url !== ""
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: frame.openLink(socialBtn.modelData.url)
                        Accessible.role: Accessible.Button
                        Accessible.name: socialBtn.modelData.tip
                        Accessible.onPressAction: if (socialBtn.modelData.url !== "") frame.openLink(socialBtn.modelData.url)
                    }
                    Tooltip { text: socialBtn.modelData.url !== "" ? socialBtn.modelData.tip : words.t("settings-supporters-coming-soon") }
                }
            }
        }
    }
}
