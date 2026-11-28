// About episode (#1562): the file behind an episode or extra row, from the right-click menu.
// Its name and folder, its size, the season and number the library read it as (or the extra
// it read), and the torrent it came from when the torrent client listed it. Copy path and
// Open folder at the foot. Over a shield of its own, on the frame's escape stack. It holds
// the keyboard while open, so Return never plays the row behind it and Tab walks only its
// own buttons, and gives it back to what had it on close.
import QtQuick
import com.marcusrosado.Anibeam

Item {
    id: root
    objectName: "about-episode-sheet"
    anchors.fill: parent
    visible: false
    property string heading: ""
    property string rowTitle: ""
    property var file: ({})
    property bool extra: false
    // The page is a film's: film words, and no season or episode rows
    property bool film: false
    readonly property real pad: theme.space(6)
    readonly property string path: String(file.path || "")
    readonly property string folder: path.replace(/[\\/][^\\/]*$/, "")
    property Item prevFocus: null
    // The value a press is on, while it is held: a drag there selects words, so the
    // panel's Flickable must not take it as a flick
    property Item pressedValue: null

    // `ep` is the row's record: an episode's, or an extra's with its label
    function openFor(ep, title, isExtra, isFilm) {
        root.file = ep || {}
        root.rowTitle = title || ""
        root.extra = !!isExtra
        root.film = !isExtra && !!isFilm
        root.heading = words.t(isExtra ? "frame-about-file" : root.film ? "frame-about-film" : "frame-about-episode")
        root.prevFocus = frame.hostWindow ? frame.hostWindow.activeFocusItem : null
        root.visible = true
        frame.escapeStack.push("sheet", root)
        panel.forceActiveFocus()
    }
    function close() {
        if (!root.visible) return
        root.visible = false
        root.pressedValue = null
        frame.escapeStack.pop(root)
        frame.focusPage(root, root.prevFocus)
        root.prevFocus = null
    }
    // Tab and Shift+Tab walk the sheet's own buttons round, never out to the page behind
    function step(dir) {
        var stops = [closeButton, openFolderButton, copyPathButton].filter(function(b) { return b.visible && b.enabled })
        if (!stops.length) return
        var at = -1
        for (var i = 0; i < stops.length; i++) if (stops[i].activeFocus) at = i
        var next = at < 0 ? (dir > 0 ? 0 : stops.length - 1) : (at + dir + stops.length) % stops.length
        stops[next].forceActiveFocus(dir > 0 ? Qt.TabFocusReason : Qt.BacktabFocusReason)
    }
    Component.onDestruction: if (visible) frame.escapeStack.pop(root)

    // The facts, one row each, in reading order; a fact the record lacks is left out
    readonly property var facts: {
        var f = root.file, out = []
        if (!root.path) return out
        // An episode with a title of its own is named by it; the rows below number it
        if (!root.extra && !root.film && f.title && root.rowTitle)
            out.push({ key: "title", label: words.t("about-episode-title"), value: root.rowTitle })
        out.push({ key: "name", label: words.t("about-episode-file-name"), value: Fmt.baseName(root.path) })
        out.push({ key: "folder", label: words.t("about-episode-folder"), value: root.folder })
        if (f.size > 0) out.push({ key: "size", label: words.t("about-episode-size"), value: words.bytes(f.size) })
        if (root.extra) {
            // The extra as the page names it (Opening 1), not the core's code
            if (root.rowTitle) out.push({ key: "extra", label: words.t("about-episode-extra"), value: root.rowTitle })
        } else if (!root.film && f.number !== undefined && f.number !== null) {
            out.push({ key: "season", label: words.t("about-episode-season"),
                       value: f.season !== undefined && f.season !== null ? String(f.season) : words.t("about-episode-no-season") })
            out.push({ key: "episode", label: words.t("about-episode-episode"), value: String(words.episodeNumber(f.number)) })
        }
        if (f.torrent) out.push({ key: "torrent", label: words.t("about-episode-torrent"), value: String(f.torrent) })
        return out
    }

    // The shield: a press outside closes it, like the other sheets
    Blocker { anchors.fill: parent; dismissOnPress: true; onDismissed: root.close() }

    Corner {
        id: panel
        anchors.centerIn: parent
        width: Math.min(theme.space(150), root.width - theme.space(16))
        height: Math.min(body.implicitHeight + root.pad * 2, root.height - theme.space(16))
        color: theme.glassFloat
        borderColor: theme.lineStrong
        borderWidth: 1
        highlight: theme.glassHighlight
        clip: true
        // Every key the sheet does not use stops here, so none reaches the page behind. Tab is
        // caught on each button and value too: the focus chain moves on from the item that
        // has the keyboard before the key would reach this panel.
        Keys.onTabPressed: function(e) { root.step(1); e.accepted = true }
        Keys.onBacktabPressed: function(e) { root.step(-1); e.accepted = true }
        Keys.onEscapePressed: root.close()
        Keys.onPressed: function(e) { e.accepted = true }
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true; onWheel: function(w) { w.accepted = true } }

        Flickable {
            id: flick
            anchors.fill: parent
            contentWidth: width
            contentHeight: body.implicitHeight + root.pad * 2
            interactive: contentHeight > height && !root.pressedValue
            boundsBehavior: Flickable.StopAtBounds
            WheelStep { flick: flick }

            Column {
                id: body
                x: root.pad; y: root.pad
                width: panel.width - root.pad * 2
                spacing: theme.space(4)

                Item {
                    width: parent.width
                    height: Math.max(head.height, closeButton.height)
                    Column {
                        id: head
                        width: parent.width - closeButton.width - theme.space(4)
                        Label { width: parent.width; text: root.heading; role: "title"; elide: Text.ElideRight }
                    }
                    Button {
                        id: closeButton
                        objectName: "about-episode-close"
                        anchors.right: parent.right
                        anchors.verticalCenter: head.verticalCenter
                        flat: true
                        icon: "x"
                        text: ""
                        accessibleName: words.t("action-close")
                        onClicked: root.close()
                        Keys.onTabPressed: function(e) { root.step(1); e.accepted = true }
                        Keys.onBacktabPressed: function(e) { root.step(-1); e.accepted = true }
                    }
                }

                Column {
                    width: parent.width
                    spacing: theme.space(3)
                    Repeater {
                        model: root.facts
                        Item {
                            required property var modelData
                            objectName: "about-episode-" + modelData.key
                            width: parent.width
                            height: Math.max(name.implicitHeight, value.implicitHeight)
                            Label {
                                id: name
                                width: theme.space(28)
                                text: modelData.label
                                role: "ui"; color: theme.textDim
                                elide: Text.ElideRight
                            }
                            // Selectable, so a part of a name can be copied as well as the path
                            TextEdit {
                                id: value
                                x: name.width + theme.space(4)
                                width: parent.width - x
                                text: modelData.value
                                readOnly: true
                                selectByMouse: true
                                Keys.onTabPressed: function(e) { root.step(1); e.accepted = true }
                                Keys.onBacktabPressed: function(e) { root.step(-1); e.accepted = true }
                                wrapMode: TextEdit.WrapAtWordBoundaryOrAnywhere
                                textFormat: TextEdit.PlainText
                                color: theme.text
                                selectionColor: theme.accentSoft
                                selectedTextColor: theme.text
                                font.family: theme.roleFamily("ui")
                                font.pointSize: theme.roleSize("ui")
                                font.weight: theme.roleWeight("ui")
                                font.variableAxes: theme.roleAxes("ui")
                                PointHandler {
                                    id: valuePress
                                    acceptedButtons: Qt.LeftButton
                                    onActiveChanged: {
                                        if (active) root.pressedValue = value
                                        else if (root.pressedValue === value) root.pressedValue = null
                                    }
                                }
                            }
                        }
                    }
                }

                Rectangle { width: parent.width; height: 1; color: theme.line }
                Item {
                    width: parent.width
                    height: theme.controlHeight
                    Row {
                        anchors.right: parent.right
                        spacing: theme.space(2)
                        Button {
                            id: openFolderButton
                            objectName: "about-episode-open-folder"
                            flat: true
                            icon: "folder-open"
                            msg: "frame-open-folder"
                            enabled: root.folder !== ""
                            onClicked: frame.openLink(Fmt.fileUrl(root.folder))
                            Keys.onTabPressed: function(e) { root.step(1); e.accepted = true }
                            Keys.onBacktabPressed: function(e) { root.step(-1); e.accepted = true }
                        }
                        Button {
                            id: copyPathButton
                            objectName: "about-episode-copy-path"
                            primary: true
                            icon: "copy"
                            msg: "frame-copy-path"
                            enabled: root.path !== ""
                            onClicked: frame.copyText(root.path, words.t("frame-path-copied"))
                            Keys.onTabPressed: function(e) { root.step(1); e.accepted = true }
                            Keys.onBacktabPressed: function(e) { root.step(-1); e.accepted = true }
                        }
                    }
                }
            }
        }
    }
}
