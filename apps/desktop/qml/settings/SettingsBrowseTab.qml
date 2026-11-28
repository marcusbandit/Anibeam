// Browse: settings for Airing now and browse views (Out now layout, Out now grid,
// and shows hidden from Airing now).
import QtQuick
import QtCore
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-browse-title")

    // Out now on Airing: cards or a list, and the cards' grid, columns by rows; AiringPage reads the same keys
    Settings { id: airingKept; location: Shell.settingsUrl; category: "airing"; property string outLayout: "4x2"; property string outStyle: "cards" }
    readonly property var outLayouts: ["3x2", "4x2", "5x2", "4x3"]

    // How many shows Airing now's not interested marks hide (#202); the list and the way back
    // for each are on their own page, SettingsHiddenAiringTab.qml
    property int notInterestedCount: 0
    function reloadNotInterested() { var r = Door.listNotInterested(); if (!r.error) notInterestedCount = r.reply.shows.length }
    Component.onCompleted: reloadNotInterested()

    Panel {
        objectName: "settings-browse-airing"
        title: words.t("settings-browse-airing")
        SettingRow { label: words.t("settings-appearance-out-now")
            Seg { objectName: "out-now-style"; options: [words.t("settings-appearance-out-now-cards"), words.t("settings-appearance-out-now-list")]; index: airingKept.outStyle === "list" ? 1 : 0; onPicked: function(i) { airingKept.outStyle = i === 1 ? "list" : "cards" } } }
        SettingRow { visible: airingKept.outStyle !== "list"; label: words.t("settings-appearance-out-now-grid")
            Seg { objectName: "out-now-layout"; options: tab.outLayouts.map(function(l) { var cr = l.split("x"); return words.t("settings-appearance-grid", { columns: Number(cr[0]), rows: Number(cr[1]) }) }); index: Math.max(0, tab.outLayouts.indexOf(airingKept.outLayout)); onPicked: function(i) { airingKept.outLayout = tab.outLayouts[i] } } }
        SettingRow {
            objectName: "settings-hidden-from-airing"
            label: words.t("settings-library-hidden-from-airing")
            Button {
                objectName: "settings-open-hidden-from-airing"
                text: tab.notInterestedCount > 0 ? words.t("count-shows", { count: tab.notInterestedCount }) : words.t("settings-library-hidden-from-airing-empty")
                icon: "eye-off"; flat: true; small: true
                enabled: tab.notInterestedCount > 0
                onClicked: page.open("hidden")
            }
        }
    }
}
