// Interactive hitbox debug overlay: outlines every live MouseArea, text input, and scroll
// container in the window without intercepting any pointer or keyboard events. Toggle with
// Ctrl+Shift+H anywhere (including over Onboarding) or from Settings > Developer.
import QtQuick
import com.marcusrosado.Anibeam

Item {
    id: overlay
    objectName: "hitbox-overlay"
    anchors.fill: parent
    z: 9999
    // Never swallow clicks, hovers, or focus
    enabled: false

    property var targetRoot: null
    property var boxes: []
    property int interactiveCount: 0

    function isVisibleInTree(item) {
        var cur = item
        while (cur) {
            if (cur === overlay) return false
            if (cur.visible === false || cur.opacity <= 0.001) return false
            cur = cur.parent
        }
        return true
    }

    function labelFor(item) {
        if (item.objectName && item.objectName !== "") return item.objectName
        if (item.parent && item.parent.objectName && item.parent.objectName !== "") return item.parent.objectName
        if (item.Accessible && item.Accessible.name && item.Accessible.name !== "") return item.Accessible.name
        return ""
    }

    function scan() {
        if (!visible || !targetRoot) { boxes = []; interactiveCount = 0; canvas.requestPaint(); return }
        var found = []
        var count = 0
        // When a full-window modal (like Onboarding) is up, scan only that top layer so
        // obscured hitboxes from the page behind its Blocker don't clutter the view.
        var scanStart = targetRoot
        if (targetRoot.onboardingPanel && targetRoot.onboardingPanel.visible) scanStart = targetRoot.onboardingPanel
        var stack = [scanStart]
        var maxW = overlay.width
        var maxH = overlay.height

        while (stack.length > 0) {
            var it = stack.pop()
            if (!it || it === overlay || it.visible === false || it.opacity <= 0.001) continue

            var typeName = String(it)
            var kind = ""
            if (typeName.indexOf("QQuickMouseArea") === 0) {
                if (it.enabled !== false) kind = "mouse"
            } else if (typeName.indexOf("QQuickTextInput") === 0 || typeName.indexOf("QQuickTextEdit") === 0) {
                if (it.enabled !== false && it.readOnly !== true) kind = "input"
            } else if (typeName.indexOf("QQuickFlickable") === 0 && it.interactive === true && it.contentHeight > it.height + 1) {
                kind = "scroll"
            }

            if (kind !== "" && it.width > 1 && it.height > 1) {
                var tl = it.mapToItem(overlay, 0, 0)
                var w = it.width
                var h = it.height
                // Skip full-window background blockers that cover the entire viewport
                var isBackdrop = (kind === "mouse" && w >= maxW - 2 && h >= maxH - 2)
                if (!isBackdrop && tl.x + w > 0 && tl.y + h > 0 && tl.x < maxW && tl.y < maxH) {
                    var hovered = (kind === "mouse" && it.containsMouse === true)
                    if (kind !== "scroll") count++
                    found.push({
                        x: Math.round(tl.x) + 0.5,
                        y: Math.round(tl.y) + 0.5,
                        w: Math.round(w),
                        h: Math.round(h),
                        kind: kind,
                        hovered: hovered,
                        label: labelFor(it)
                    })
                }
            }

            var kids = it.children
            if (kids) {
                for (var i = kids.length - 1; i >= 0; i--) stack.push(kids[i])
            }
        }

        // Draw scroll containers first, then normal hitboxes, then hovered hitboxes on top
        found.sort(function(a, b) {
            var pa = a.hovered ? 2 : a.kind === "scroll" ? 0 : 1
            var pb = b.hovered ? 2 : b.kind === "scroll" ? 0 : 1
            return pa - pb
        })
        boxes = found
        interactiveCount = count
        canvas.requestPaint()
    }

    Timer {
        interval: 150
        repeat: true
        running: overlay.visible
        triggeredOnStart: true
        onTriggered: overlay.scan()
    }
    onWidthChanged: if (visible) scan()
    onHeightChanged: if (visible) scan()
    onVisibleChanged: if (visible) scan()

    Canvas {
        id: canvas
        anchors.fill: parent
        renderStrategy: Canvas.Cooperative
        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            var list = overlay.boxes
            if (!list || list.length === 0) return

            ctx.font = "600 10px monospace" // i18n: none
            ctx.textBaseline = "top"

            for (var i = 0; i < list.length; i++) {
                var b = list[i]
                var stroke = b.hovered ? "#4ade80"
                           : b.kind === "input" ? "#c084fc"
                           : b.kind === "scroll" ? "rgba(251, 191, 36, 0.55)" // i18n: none
                           : "rgba(56, 189, 248, 0.85)" // i18n: none
                var fill = b.hovered ? "rgba(74, 222, 128, 0.16)" // i18n: none
                         : b.kind === "input" ? "rgba(192, 132, 252, 0.10)" // i18n: none
                         : b.kind === "scroll" ? "transparent"
                         : "rgba(56, 189, 248, 0.07)" // i18n: none

                ctx.save()
                if (b.kind === "scroll") ctx.setLineDash([5, 4])
                ctx.lineWidth = b.hovered ? 2 : 1
                ctx.strokeStyle = stroke
                if (fill !== "transparent") {
                    ctx.fillStyle = fill
                    ctx.fillRect(b.x, b.y, b.w, b.h)
                }
                ctx.strokeRect(b.x, b.y, b.w, b.h)
                ctx.restore()

                // Size + name tag: always on hovered, and compact size on reasonably sized hitboxes
                if (b.hovered || (b.kind !== "scroll" && b.w >= 48 && b.h >= 22)) {
                    var dims = b.w + "x" + b.h
                    var tag = b.hovered && b.label !== "" ? (b.label + "  " + dims) : dims
                    var tw = ctx.measureText(tag).width + 8
                    var th = 14
                    var tx = Math.max(2, Math.min(width - tw - 2, b.x))
                    var ty = b.y >= th + 2 ? b.y - th - 1 : b.y + 2
                    ctx.fillStyle = b.hovered ? "rgba(10, 14, 20, 0.92)" : "rgba(10, 14, 20, 0.72)" // i18n: none
                    ctx.fillRect(tx, ty, tw, th)
                    ctx.fillStyle = b.hovered ? "#4ade80" : stroke
                    ctx.fillText(tag, tx + 4, ty + 2)
                }
            }
        }
    }

    // Quiet top-center status pill so debug mode is unmistakable
    Rectangle {
        anchors.top: parent.top; anchors.topMargin: theme.space(2)
        anchors.horizontalCenter: parent.horizontalCenter
        height: theme.space(6.5)
        width: badgeRow.implicitWidth + theme.space(5)
        radius: height / 2
        color: Qt.rgba(0.05, 0.07, 0.10, 0.9)
        border.width: 1; border.color: "#38bdf8"
        Row {
            id: badgeRow
            anchors.centerIn: parent
            spacing: theme.space(2)
            Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 7; height: 7; radius: 3.5; color: "#38bdf8" }
            Label {
                anchors.verticalCenter: parent.verticalCenter
                role: "figure"
                font.pointSize: theme.typeSmall * 0.9
                color: "#e2f2ff"
                text: "HITBOXES · " + overlay.interactiveCount + " · Ctrl+Shift+H" // i18n: none
            }
        }
    }
}
