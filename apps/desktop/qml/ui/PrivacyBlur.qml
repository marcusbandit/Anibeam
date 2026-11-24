// Blurs sensitive content when Privacy mode is on (frame.prefs.privacyMode).
// Callers pass `blurred ? mask(realText) : realText` so the text behind the blur
// is a deterministic random decoy string (no solid block, and unrecoverable from
// screenshots). Hover does nothing; an active click opens a compact "Show content"
// popover with an eye icon in frame.overlay. Clicking that reveals the real content
// until the pointer moves away, or the tab/page changes.
import QtQuick
import QtQuick.Effects
import com.marcusrosado.Anibeam

Item {
    id: root
    // Set to false on rows that carry no sensitive data so SettingRow can keep one tree
    property bool active: true
    readonly property bool privacyOn: active && typeof frame !== "undefined" && frame && frame.prefs && frame.prefs.privacyMode === true
    property bool revealed: false
    readonly property bool blurred: privacyOn && !revealed

    default property alias content: host.data

    implicitWidth: host.implicitWidth
    implicitHeight: host.implicitHeight

    // Replaces letters and digits with a deterministic pseudo-random decoy string of
    // the same shape so blurred text looks natural without exposing real characters.
    function mask(value) {
        var s = String(value || "")
        if (s === "") return ""
        var seed = 2166136261
        for (var i = 0; i < s.length; i++) {
            seed ^= s.charCodeAt(i)
            seed = (seed * 16777619) >>> 0
        }
        var lower = "abcdefghjkmnpqrstuvwxyz" // i18n: none
        var upper = "ABCDEFGHJKMNPQRSTUVWXYZ" // i18n: none
        var digits = "0123456789" // i18n: none
        var out = ""
        for (var j = 0; j < s.length; j++) {
            seed = (seed * 1664525 + 1013904223) >>> 0
            var c = s.charCodeAt(j)
            if (c >= 97 && c <= 122) out += lower[seed % lower.length]
            else if (c >= 65 && c <= 90) out += upper[seed % upper.length]
            else if (c >= 48 && c <= 57) out += digits[seed % digits.length]
            else out += s[j]
        }
        return out
    }

    function rehide() {
        leaveCheck.stop()
        if (revealed) revealed = false
        if (typeof frame !== "undefined" && frame && frame.closePrivacyReveal) frame.closePrivacyReveal(root)
    }

    onPrivacyOnChanged: rehide()
    onVisibleChanged: if (!visible) rehide()
    Connections {
        target: typeof frame !== "undefined" && frame ? frame : null
        function onSettingsTabChanged() { root.rehide() }
    }
    Connections {
        target: typeof frame !== "undefined" && frame && frame.nav ? frame.nav : null
        function onCurrentChanged() { root.rehide() }
    }
    Component.onDestruction: {
        if (typeof frame !== "undefined" && frame && frame.closePrivacyReveal) frame.closePrivacyReveal(root)
    }

    // The wrapped content. Hidden from direct paint while MultiEffect renders it blurred.
    Item {
        id: host
        anchors.fill: parent
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
        visible: !root.blurred
    }

    // Real Gaussian blur over the decoy string via QtQuick.Effects.MultiEffect.
    Loader {
        id: blurLoader
        anchors.fill: parent
        active: root.blurred && root.width > 0 && root.height > 0
        sourceComponent: MultiEffect {
            source: host
            blurEnabled: true
            blur: 0.75
            blurMax: 18
            autoPaddingEnabled: true
        }
    }

    // Active click gate while blurred: hovering does nothing; clicking asks to show content.
    MouseArea {
        id: clickGate
        anchors.fill: parent
        visible: root.blurred
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        preventStealing: true
        Accessible.role: Accessible.Button
        Accessible.name: words.t("privacy-blurred-content")
        Accessible.onPressAction: clickGate.clicked(null)
        onClicked: function(mouse) {
            if (typeof frame !== "undefined" && frame && frame.askPrivacyReveal) {
                frame.askPrivacyReveal(root, function() {
                    root.revealed = true
                    leaveCheck.restart()
                })
            } else {
                root.revealed = true
            }
        }
    }

    // Once revealed, moving the pointer away hides it again.
    HoverHandler {
        id: revealHover
        enabled: root.privacyOn && root.revealed
        onHoveredChanged: {
            if (hovered) leaveCheck.stop()
            else if (!leaveCheck.running) root.rehide()
        }
    }
    Timer {
        id: leaveCheck
        interval: 900
        onTriggered: {
            if (root.revealed && !revealHover.hovered) root.rehide()
        }
    }
}
