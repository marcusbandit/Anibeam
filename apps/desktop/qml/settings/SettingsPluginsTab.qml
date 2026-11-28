// Plugins: placeholder tab for upcoming plugin settings.
import QtQuick
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-plugins-title")

    Panel {
        SettingRow {
            label: words.t("settings-plugins-empty")
            line: words.t("settings-plugins-empty-line")
        }
    }
}
