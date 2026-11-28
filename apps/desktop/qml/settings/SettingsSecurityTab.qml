// Security and privacy: Privacy mode (blurs sensitive fields, paths, and addresses across
// the app).
import QtQuick
import com.marcusrosado.Anibeam

SettingsTab {
    id: tab
    title: words.t("settings-security-title")

    Panel {
        objectName: "settings-privacy"
        title: words.t("settings-privacy-title")
        SettingRow {
            label: words.t("settings-privacy-mode")
            line: words.t("settings-privacy-mode-line")
            Switch { objectName: "privacy-mode"; checked: frame.prefs.privacyMode; onToggled: function(on) { frame.prefs.privacyMode = on } }
        }
    }
}
