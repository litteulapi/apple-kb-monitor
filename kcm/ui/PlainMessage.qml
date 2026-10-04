// SPDX-License-Identifier: GPL-2.0-or-later
//
// InlineMessage shown as plain text: its messages embed daemon and BlueZ strings (a keyboard name may hold markup).
// Kirigami's label has no textFormat property of the message; the pages are built before any daemon reply.
import QtQuick
import org.kde.kirigami as Kirigami

Kirigami.InlineMessage {
    id: message

    function plain(item) {
        for (const c of item.children) {
            if (c.textFormat !== undefined && c.text !== undefined) {
                c.textFormat = Text.PlainText;
                // re-read: a text already parsed as markup would keep its rich document
                c.text = Qt.binding(function () { return message.text; });
                return true;
            }
            if (plain(c)) return true;
        }
        return false;
    }
    Component.onCompleted: {
        if (!plain(message)) console.warn("apple-kb-monitor: no text item in InlineMessage, markup not disabled");
    }
}
