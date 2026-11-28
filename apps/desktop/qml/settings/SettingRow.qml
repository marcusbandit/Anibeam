// One setting: an optional leading icon (`icon`), a label, an optional line in the fixed
// face under it (`line`), an optional result line under that (`status`), and a control slot
// on the right. Setting `clickable: true` makes the whole row a clean interactive list item
// with hover wash and an optional `trailingIcon` (e.g. "chevron-right" or "external-link"),
// avoiding repetitive "Open" buttons.
import QtQuick
import QtQuick.Layouts
import com.marcusrosado.Anibeam

Item {
    id: root
    property string label: ""
    property string line: ""
    property string status: ""
    property string icon: ""
    property string trailingIcon: ""
    property bool clickable: false
    property bool rowEnabled: true
    property bool redactLabel: false
    property bool redactLine: false
    property bool redactStatus: false
    property bool hairline: true
    signal clicked()
    readonly property bool settingTarget: true
    property real flash: 0
    function pulse() { flashAnim.restart() }
    SequentialAnimation {
        id: flashAnim
        NumberAnimation { target: root; property: "flash"; to: 1; duration: 160 }
        PauseAnimation { duration: 700 }
        NumberAnimation { target: root; property: "flash"; to: 0; duration: 600 }
    }
    default property alias control: slot.data

    readonly property real pad: theme.space(2.5)
    readonly property real gutter: theme.space(6)
    readonly property real leadWidth: icon !== "" ? theme.space(8.5) : 0
    readonly property real rightWidth: Math.max(slot.width, trailingGlyph.visible ? trailingGlyph.width : 0)
    readonly property bool wordy: line !== "" || status !== ""
    readonly property bool stacked: wordy && !clickable && width - leadWidth - rightWidth - gutter < theme.space(60)
    width: parent ? parent.width : theme.space(100)
    implicitHeight: stacked ? textColumn.implicitHeight + pad + slot.height + pad * 2
                            : Math.max(theme.space(12), textColumn.implicitHeight + pad * 2, slot.height + pad * 2)
    Layout.fillWidth: true
    opacity: clickable && !rowEnabled ? theme.disabledOpacity : 1

    // Hover wash for clickable rows
    Rectangle {
        visible: root.clickable && root.rowEnabled && rowMouse.containsMouse
        x: -theme.space(3); width: root.width + theme.space(6)
        y: 1; height: root.height - 2
        radius: theme.radiusSm
        color: theme.raised
    }
    // The search highlight: a soft accent wash under the row, reaching past its sides into
    // the pane's padding
    Rectangle {
        visible: root.flash > 0
        x: -theme.space(3); width: root.width + theme.space(6)
        y: 1; height: root.height - 2
        radius: theme.radiusSm
        color: theme.accentSoft
        opacity: root.flash
    }
    Icon {
        visible: root.icon !== ""
        x: 0
        y: Math.round((root.height - height) / 2)
        glyph: root.icon
        size: theme.space(5)
        color: root.clickable && root.rowEnabled && rowMouse.containsMouse ? theme.accent : theme.textDim
    }
    Column {
        id: textColumn
        x: root.leadWidth
        y: root.stacked ? root.pad : Math.round((root.height - height) / 2)
        width: root.stacked ? Math.max(0, root.width - root.leadWidth) : Math.max(0, root.width - root.leadWidth - root.rightWidth - root.gutter)
        spacing: theme.space(0.5)
        PrivacyBlur {
            id: labelBlur
            active: root.redactLabel
            width: Math.min(labelText.implicitWidth, parent.width)
            height: labelText.implicitHeight
            Text {
                id: labelText
                width: textColumn.width
                text: labelBlur.blurred ? labelBlur.mask(root.label) : root.label
                color: theme.text
                wrapMode: Text.Wrap
                font.family: theme.fontSans
                font.pointSize: theme.typeNormal
            }
        }
        PrivacyBlur {
            id: lineBlur
            visible: root.line !== ""
            active: root.redactLine
            width: Math.min(lineText.implicitWidth, parent.width)
            height: lineText.implicitHeight
            Text {
                id: lineText
                width: textColumn.width
                text: lineBlur.blurred ? lineBlur.mask(root.line) : root.line
                color: theme.textDim
                wrapMode: Text.Wrap
                font.family: theme.fontUi
                font.pointSize: theme.typeSmall
            }
        }
        PrivacyBlur {
            id: statusBlur
            visible: root.status !== ""
            active: root.redactStatus
            width: Math.min(statusText.implicitWidth, parent.width)
            height: statusText.implicitHeight
            Text {
                id: statusText
                width: textColumn.width
                text: statusBlur.blurred ? statusBlur.mask(root.status) : root.status
                color: theme.textDim
                wrapMode: Text.Wrap
                font.family: theme.fontUi
                font.pointSize: theme.typeSmall
            }
        }
    }
    Icon {
        id: trailingGlyph
        visible: root.trailingIcon !== ""
        x: root.width - width
        y: Math.round((root.height - height) / 2)
        glyph: root.trailingIcon
        size: theme.space(4.5)
        color: root.clickable && root.rowEnabled && rowMouse.containsMouse ? theme.accent : theme.textFaint
    }
    Item {
        id: slot
        x: root.width - width
        y: root.stacked ? root.height - root.pad - height : Math.round((root.height - height) / 2)
        width: childrenRect.width
        height: childrenRect.height
    }
    MouseArea {
        id: rowMouse
        visible: root.clickable
        enabled: root.clickable && root.rowEnabled
        x: -theme.space(3); width: root.width + theme.space(6)
        y: 0; height: root.height
        hoverEnabled: true
        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: root.clicked()
        Accessible.role: Accessible.Button
        Accessible.name: root.label
        Accessible.onPressAction: if (root.rowEnabled) root.clicked()
    }
}
