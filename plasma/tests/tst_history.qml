import QtQuick
import "../com.agenceapi.devicehub/contents/ui/History.js" as Hist
import "../com.agenceapi.devicehub/contents/ui/FnMode.js" as Fn

Item {
    function check(name, ok) {
        if (!ok) { console.log("FAIL history: " + name); Qt.exit(1); }
    }
    Component.onCompleted: {
        var now = 1790858400;
        var all = [];
        for (var i = 0; i < 259200; i++)
            all.push({ ts: now - 30 * (259199 - i), pct: 100 - 60 * i / 259200, voltage: 2.4 + 0.6 * (1 - i / 259200) });
        var days = [7, 30, 90];
        var counts = [20161, 86401, 259200];
        for (var d = 0; d < days.length; d++) {
            var s = Hist.series(all, now - days[d] * 86400, now, Hist.POINTS_MAX);
            check(days[d] + " d count " + s.count, s.count === counts[d]);
            check(days[d] + " d bounded " + s.pct.length, s.pct.length <= Hist.POINTS_MAX && s.pct.length > Hist.POINTS_MAX / 2);
            check(days[d] + " d voltage bounded", s.volt.length <= Hist.POINTS_MAX);
            check(days[d] + " d ends kept", s.pct[0][0] === s.tMin && s.pct[s.pct.length - 1][0] === s.tMax && s.tMax === now);
            var sorted = true;
            for (var k = 1; k < s.pct.length; k++) sorted = sorted && s.pct[k - 1][0] <= s.pct[k][0];
            check(days[d] + " d sorted", sorted);
        }
        var flat = [];
        for (var j = 0; j < 50000; j++) flat.push([j, 50]);
        flat[12345] = [12345, 3];
        flat[40000] = [40000, 99];
        var ds = Hist.downsample(flat, 240);
        var lo = 100, hi = 0;
        for (var m = 0; m < ds.length; m++) { lo = Math.min(lo, ds[m][1]); hi = Math.max(hi, ds[m][1]); }
        check("downsample size " + ds.length, ds.length <= 240);
        check("extremes kept", lo === 3 && hi === 99);
        check("ends kept", ds[0][0] === 0 && ds[ds.length - 1][0] === 49999);
        check("small untouched", Hist.downsample(flat.slice(0, 200), 240).length === 200);
        for (var mx = 4; mx <= 9; mx++)
            for (var len = mx + 1; len < 4 * mx; len++)
                check("bound " + mx + "/" + len, Hist.downsample(flat.slice(0, len), mx).length <= mx);

        // Invalid readings are dropped, never drawn.
        var bad = [
            { ts: now - 100, pct: 80, voltage: 2.9 },
            { ts: now - 90, pct: 150 },                               // not a percentage
            { ts: now - 80, pct: NaN },
            { ts: now - 70, pct: 79, voltage: 2.9, voltage_valid: false }, // legacy voltage
            { ts: now - 60, pct: 78, voltage: 1e300 },
            { ts: now + 30 * 86400, pct: 10 },                        // dated in the future
            { ts: now - 8 * 86400, pct: 5 },                          // before the period
            { ts: "x", pct: 50 }, null, 7,
            { ts: now - 50, pct: 77, voltage: 2.8 }
        ];
        var b = Hist.series(bad, now - 7 * 86400, now, 240);
        check("valid count " + b.count, b.count === 4);
        check("measured voltages only " + b.volt.length, b.volt.length === 2 && b.voltMin === 2.8 && b.voltMax === 2.9);
        check("time axis not stretched", b.tMax === now - 50);

        var e1 = Hist.parse("not json", 0, now, 240);
        var e2 = Hist.parse("{\"a\":1}", 0, now, 240);
        var e3 = Hist.parse("[]", 0, now, 240);
        check("garbage", e1.count === 0 && e2.count === 0 && e3.count === 0 && e1.pct.length === 0 && isNaN(e3.voltMin));
        check("parse", Hist.parse(JSON.stringify(bad), now - 7 * 86400, now, 240).count === 4);

        check("fn 1->2", Fn.next(1) === 2 && Fn.next(3) === 2);
        check("fn 2->1", Fn.next(2) === 1);
        check("fn 0/4 -> default", Fn.next(0) === 1 && Fn.next(4) === 1);
        check("fn unknown", Fn.next(-1) === -1 && Fn.next(-100) === -1 && Fn.next(7) === -1);
        check("fn kinds", Fn.kind(1) === "media" && Fn.kind(3) === "media" && Fn.kind(2) === "fkeys" && Fn.kind(0) === "off" && Fn.kind(4) === "nofkeys" && Fn.kind(-100) === "");
        check("device path", Fn.devicePath("/com/agenceapi/AppleKbMonitor1", "aa:bb:cc:dd:ee:f1") === "/com/agenceapi/AppleKbMonitor1/devices/AA_BB_CC_DD_EE_F1");
        check("bad address", Fn.devicePath("/x", "AA:BB") === "" && Fn.devicePath("/x", "../../etc") === "" && Fn.devicePath("/x", "") === "");
        console.log("PASS history");
        Qt.exit(0);
    }
}
