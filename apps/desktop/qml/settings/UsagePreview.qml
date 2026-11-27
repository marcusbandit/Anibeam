// The usage log before it is sent (#1532): the exact text GetUsage reads, in a scrolling
// fixed-width view, with Send and Cancel. A raised tile over a Blocker the About tab puts
// under it, on the frame's escape stack like the other sheets. Send is the tab's: it saves
// the file and then sends it.
import QtQuick
import QtQuick.Controls.Basic as QC
import com.marcusrosado.Anibeam

Corner {
    id: root
    objectName: "usage-preview"
    property string text: ""
    property bool sending: false
    signal send()

    function openWith(text) {
        root.text = text
        textFlick.contentY = 0
        if (visible) return
        visible = true
        frame.escapeStack.push("popover", root)
        root.forceActiveFocus()
    }
    function close() {
        if (!visible) return
        visible = false
        frame.escapeStack.pop(root)
    }
    Component.onDestruction: if (visible) frame.escapeStack.pop(root)

    readonly property real pad: theme.space(6)
    width: Math.min(theme.space(170), (parent ? parent.width : theme.space(170)) - theme.space(16))
    height: Math.min(theme.space(170), (parent ? parent.height : theme.space(170)) - theme.space(16))
    visible: false
    color: theme.glassFloat
    borderColor: theme.lineStrong
    borderWidth: 1
    highlight: theme.glassHighlight

    // The panel's own press sink: a press on its padding never reaches the shield under it
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true; onWheel: function(w) { w.accepted = true } }

    // The keys scroll through the wheel's target, so they glide as a notch does
    Keys.onPressed: function(event) {
        if ((event.modifiers & ~Qt.KeypadModifier) !== Qt.NoModifier) return
        var k = event.key, page = textFlick.height - theme.space(10)
        var to = k === Qt.Key_Down ? wheel.target + wheel.notch
               : k === Qt.Key_Up ? wheel.target - wheel.notch
               : k === Qt.Key_PageDown || k === Qt.Key_Space ? wheel.target + page
               : k === Qt.Key_PageUp ? wheel.target - page
               : k === Qt.Key_Home ? 0
               : k === Qt.Key_End ? textFlick.contentHeight
               : undefined
        if (to === undefined) return
        wheel.scrollTo(to)
        event.accepted = true
    }

    Item {
        id: head
        x: root.pad; y: root.pad
        width: root.width - root.pad * 2
        height: Math.max(title.height, closeButton.height)
        Label { id: title; anchors.verticalCenter: parent.verticalCenter; msg: "settings-usage-preview-title"; role: "title" }
        Button {
            id: closeButton
            objectName: "usage-preview-close"
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            flat: true
            icon: "x"
            text: ""
            onClicked: root.close()
        }
    }

    Corner {
        id: well
        x: root.pad
        anchors.top: head.bottom; anchors.topMargin: theme.space(4)
        anchors.bottom: foot.top; anchors.bottomMargin: theme.space(4)
        width: root.width - root.pad * 2
        radius: theme.radiusTile
        smoothing: theme.cornerSmoothing
        color: theme.tile
        borderColor: theme.line
        borderWidth: 1
        Flickable {
            id: textFlick
            objectName: "usage-preview-text"
            anchors.fill: parent
            anchors.margins: theme.space(3)
            contentWidth: width
            contentHeight: body.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            QC.ScrollBar.vertical: ThinScrollBar {}
            WheelStep { id: wheel; flick: textFlick }
            TextEdit {
                id: body
                width: textFlick.width - theme.space(3)
                readOnly: true
                selectByMouse: true
                textFormat: TextEdit.PlainText
                wrapMode: TextEdit.WrapAtWordBoundaryOrAnywhere
                text: root.text
                color: theme.text
                font.family: "monospace"
                font.pointSize: theme.typeSmall
                selectionColor: theme.accentSoft
                selectedTextColor: theme.text
            }
        }
    }

    Row {
        id: foot
        anchors.right: parent.right; anchors.rightMargin: root.pad
        anchors.bottom: parent.bottom; anchors.bottomMargin: root.pad
        spacing: theme.space(2)
        Button { objectName: "usage-preview-cancel"; flat: true; msg: "action-cancel"; onClicked: root.close() }
        Button {
            objectName: "usage-preview-send"
            primary: true
            icon: "send"
            msg: "settings-usage-preview-send"
            enabled: !root.sending
            onClicked: root.send()
        }
    }
}
