// Supporters: ways to support Anibeam development (Patreon, Ko-fi) and get in touch on
// Discord, each as a clean branded clickable row.
import QtQuick
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-supporters-title")

    // Set once Patreon and the community Discord invite are live
    readonly property string kofiUrl: "https://ko-fi.com/marcusbandit" // i18n: none
    readonly property string patreonUrl: "" // i18n: none
    readonly property string discordUrl: "" // i18n: none

    Panel {
        title: words.t("settings-supporters-ways")
        SettingRow {
            objectName: "supporters-patreon"
            icon: "patreon"
            label: "Patreon" // i18n: none
            line: tab.patreonUrl !== "" ? words.t("settings-supporters-patreon-line") : words.t("settings-supporters-coming-soon")
            clickable: true
            rowEnabled: tab.patreonUrl !== ""
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.patreonUrl)
        }
        SettingRow {
            objectName: "supporters-kofi"
            icon: "kofi"
            label: "Ko-fi" // i18n: none
            line: words.t("settings-supporters-kofi-line")
            clickable: true
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.kofiUrl)
        }
    }
    Panel {
        title: words.t("settings-supporters-contact")
        SettingRow {
            objectName: "supporters-discord"
            icon: "discord"
            label: "Discord" // i18n: none
            line: words.t("settings-supporters-discord-line")
            clickable: true
            rowEnabled: tab.discordUrl !== ""
            trailingIcon: "external-link"
            onClicked: frame.openLink(tab.discordUrl)
        }
    }
}
