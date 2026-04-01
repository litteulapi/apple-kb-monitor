import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

MouseArea {
    id: compact
    onClicked: root.expanded = !root.expanded

    Canvas {
        id: scarabCanvas
        anchors.fill: parent

        property real pulse: 0
        property int frame: 0

        NumberAnimation on pulse {
            from: 0
            to: 1
            duration: 2000
            loops: Animation.Infinite
        }

        Timer {
            interval: 400
            running: true
            repeat: true
            onTriggered: {
                scarabCanvas.frame = (scarabCanvas.frame + 1) % 4
                scarabCanvas.requestPaint()
            }
        }

        onPulseChanged: requestPaint()

        onPaint: {
            var ctx = getContext("2d")
            var w = width
            var h = height
            var px = Math.max(1, Math.floor(Math.min(w, h) / 16))
            ctx.clearRect(0, 0, w, h)

            var batColor = "#00D4FF"
            if (!root.connected) batColor = "#444"
            else if (root.batteryPercent <= 15) batColor = "#FF4444"
            else if (root.batteryPercent <= 50) batColor = "#FFaa00"

            // Glow
            var ga = 0.1 + pulse * 0.15
            var grad = ctx.createRadialGradient(w/2, h/2, 0, w/2, h/2, w*0.6)
            grad.addColorStop(0, "rgba(0,212,255," + ga + ")")
            grad.addColorStop(1, "rgba(0,0,0,0)")
            ctx.fillStyle = grad
            ctx.fillRect(0, 0, w, h)

            // Scarab pixel art 16x16
            var S = [
                "0000001111000000",
                "0000012222100000",
                "0000123333210000",
                "0001233333321000",
                "0012333443332100",
                "0123334444333210",
                "0123344444433210",
                "1200344444430021",
                "1200344334430021",
                "0123344334332100",
                "0123334444333210",
                "0012333333332100",
                "0001233333321000",
                "0010123333210100",
                "0100012222100010",
                "1000001111000001"
            ]

            // Wing flap animation
            var wingL = ["12", "01", "00", "01"]
            var wingR = ["21", "10", "00", "10"]
            var f = frame
            S[7] = wingL[f] + "00344444430" + "0" + wingR[f]
            S[8] = wingL[f] + "00344334430" + "0" + wingR[f]

            // Palette
            var pal = [
                "rgba(0,0,0,0)",
                "rgba(0,180,220," + (0.4 + pulse * 0.4) + ")",
                batColor,
                Qt.lighter(batColor, 1.4),
                "#FFFFFF"
            ]

            var ox = Math.floor((w - 16 * px) / 2)
            var oy = Math.floor((h - 16 * px) / 2)

            for (var y = 0; y < 16; y++) {
                var row = S[y]
                for (var x = 0; x < 16; x++) {
                    var c = parseInt(row.charAt(x))
                    if (c > 0) {
                        ctx.fillStyle = pal[c]
                        ctx.fillRect(ox + x * px, oy + y * px, px, px)
                    }
                }
            }

            // Battery % text
            if (root.connected) {
                ctx.fillStyle = "#FFFFFF"
                ctx.font = "bold " + Math.max(7, px * 2.5) + "px monospace"
                ctx.textAlign = "center"
                ctx.textBaseline = "bottom"
                ctx.fillText(root.batteryPercent, w / 2, h - 1)
            }
        }
    }
}
