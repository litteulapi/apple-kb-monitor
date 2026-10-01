#!/usr/bin/env python3
"""Unit tests for apple-kb-monitor — decode, interpolation, SDP, analytics."""
import json
import struct
import sys
import unittest
from pathlib import Path

# Import functions from the main script
sys.path.insert(0, str(Path(__file__).parent.parent))

# We can't import the script directly (no .py extension), so exec it
# and extract the functions we need
_globals = {}
_src = Path(__file__).parent.parent / "apple-kb-monitor"
_code = _src.read_text()
# Only exec the function definitions, not main()
_code = _code.split("\nif __name__")[0]
try:
    exec(compile(_code, str(_src), "exec"), _globals)
except Exception:
    pass  # Some imports may fail without D-Bus, that's OK


class TestHIDDecode(unittest.TestCase):
    """Test HID Feature Report decoding."""

    def test_battery_pct(self):
        """Report 0x47 — standard battery percentage."""
        data = bytes([0x47, 100])
        self.assertEqual(data[0], 0x47)
        self.assertEqual(data[1], 100)

    def test_battery_fine(self):
        """Report 0xEA — precise battery percentage."""
        data = bytes([0xEA, 98])
        self.assertEqual(data[0], 0xEA)
        self.assertEqual(data[1], 98)

    def test_adc_voltage(self):
        """Report 0xF5 — ADC raw voltage decode."""
        # ADC raw = 924, Vref = 3.3V, 10-bit
        raw = 924
        adc_max = 1023
        voltage = round(raw / adc_max * 3.3, 3)
        self.assertAlmostEqual(voltage, 2.981, places=3)

    def test_calibration_curve(self):
        """Report 0x5A — discharge curve thresholds."""
        data = bytes([0x5A, 0x0B, 0x54, 0x09, 0x92, 0x09, 0x2E, 0x07, 0xD0])
        thresholds = []
        for i in range(4):
            off = 1 + i * 2
            thresholds.append((data[off] << 8) | data[off + 1])
        self.assertEqual(thresholds, [2900, 2450, 2350, 2000])

    def test_firmware_version(self):
        """Report 0x4F — firmware version parse."""
        data = bytes([0x4F, 0x50])
        fw = f"{data[1] >> 4}.{data[1] & 0x0F}"
        self.assertEqual(fw, "5.0")

    def test_build_revision(self):
        """Report 0xFF — build number."""
        data = bytes([0xFF, 0x0C, 0x32, 0x01])
        build = (data[1] << 8) | data[2]
        flag = data[3]
        self.assertEqual(build, 3122)
        self.assertEqual(flag, 1)

    def test_device_name(self):
        """Reports 0x51-0x53 — device name chunks."""
        c1 = bytes([0x51]) + b"Apple Wi\x00"
        c2 = bytes([0x52]) + b"reless K\x00"
        c3 = bytes([0x53]) + b"eyboard\x00"
        name = (c1[1:].split(b"\x00")[0].decode() +
                c2[1:].split(b"\x00")[0].decode() +
                c3[1:].split(b"\x00")[0].decode())
        self.assertEqual(name, "Apple Wireless Keyboard")

    def test_identity_key(self):
        """Report 0x4C — 144-bit identity key."""
        data = bytes.fromhex("4c030d7c5266946cfedf4bbfc0d0acdc57eb13c8")
        self.assertEqual(data[0], 0x4C)
        self.assertEqual(data[1], 3)  # type
        key = data[2:].hex()
        self.assertEqual(len(data[2:]), 18)  # 144 bits

    def test_bt_conn_params(self):
        """Report 0x46 — BT connection interval + latency."""
        data = bytes([0x46, 55, 12])
        interval_ms = data[1] * 1.25
        self.assertAlmostEqual(interval_ms, 68.75)
        self.assertEqual(data[2], 12)  # latency

    def test_device_state(self):
        """Report 0x09 — device state flag."""
        self.assertEqual(bytes([0x09, 1])[1], 1)  # OK
        self.assertEqual(bytes([0x09, 0])[1], 0)  # LOW

    def test_rom_mirrors(self):
        """Reports 0x60 and 0xEB should match 0x5A."""
        curve_5a = bytes.fromhex("5a0b540992092e07d0")
        mirror_60 = bytes.fromhex("600b540992092e07d0")
        mirror_eb = bytes.fromhex("eb0b540992092e07d0")
        self.assertEqual(curve_5a[1:], mirror_60[1:])
        self.assertEqual(curve_5a[1:], mirror_eb[1:])


class TestVoltageInterpolation(unittest.TestCase):
    """Test battery percentage interpolation from voltage + calibration."""

    def _interp(self, mv, thresholds=None):
        if thresholds is None:
            thresholds = [2900, 2450, 2350, 2000]
        pct_levels = [100, 75, 50, 25, 0]
        mv_levels = thresholds + [0]
        if mv >= mv_levels[0]:
            return 100
        if mv <= 0:
            return 0
        for i in range(len(mv_levels) - 1):
            if mv >= mv_levels[i + 1]:
                hi_pct, lo_pct = pct_levels[i], pct_levels[i + 1]
                hi_mv, lo_mv = mv_levels[i], mv_levels[i + 1]
                if hi_mv == lo_mv:
                    return hi_pct
                frac = (mv - lo_mv) / (hi_mv - lo_mv)
                return round(lo_pct + frac * (hi_pct - lo_pct))
        return 0

    def test_full_charge(self):
        self.assertEqual(self._interp(3000), 100)
        self.assertEqual(self._interp(2900), 100)

    def test_75_percent(self):
        self.assertEqual(self._interp(2450), 75)

    def test_50_percent(self):
        self.assertEqual(self._interp(2350), 50)

    def test_25_percent(self):
        self.assertEqual(self._interp(2000), 25)

    def test_between_100_75(self):
        # 2675 mV = midpoint between 2900 and 2450
        result = self._interp(2675)
        self.assertGreater(result, 75)
        self.assertLess(result, 100)

    def test_between_50_25(self):
        result = self._interp(2175)
        self.assertGreater(result, 25)
        self.assertLess(result, 50)

    def test_dead_battery(self):
        self.assertEqual(self._interp(0), 0)

    def test_very_low(self):
        result = self._interp(500)
        self.assertGreater(result, 0)
        self.assertLess(result, 25)


class TestBatteryTypeDetection(unittest.TestCase):
    """Test battery chemistry detection from voltage."""

    def _detect(self, voltage):
        if voltage is None:
            return {"type": "unknown", "confidence": 0}
        if voltage >= 3.1:
            return {"type": "lithium_fresh", "confidence": 80}
        if voltage >= 2.85:
            return {"type": "alkaline_fresh", "confidence": 70}
        if voltage >= 2.5:
            return {"type": "alkaline_or_nimh", "confidence": 40}
        if 2.3 <= voltage < 2.5:
            return {"type": "nimh_likely", "confidence": 60}
        if 2.0 <= voltage < 2.3:
            return {"type": "depleted", "confidence": 80}
        return {"type": "critical", "confidence": 90}

    def test_fresh_alkaline(self):
        r = self._detect(2.981)
        self.assertEqual(r["type"], "alkaline_fresh")

    def test_nimh(self):
        r = self._detect(2.4)
        self.assertEqual(r["type"], "nimh_likely")

    def test_dead(self):
        r = self._detect(1.5)
        self.assertEqual(r["type"], "critical")

    def test_none(self):
        r = self._detect(None)
        self.assertEqual(r["type"], "unknown")


class TestSDPDecode(unittest.TestCase):
    """Test SDP element parsing."""

    def test_uint8(self):
        # Type 1 (unsigned int), size 0 (1 byte) = header 0x08
        data = bytes([0x08, 42])
        header = data[0]
        dtype = (header >> 3) & 0x1F
        dsize = header & 0x07
        self.assertEqual(dtype, 1)  # unsigned int
        self.assertEqual(dsize, 0)  # 1 byte
        self.assertEqual(data[1], 42)

    def test_uint16(self):
        # Type 1 (unsigned int), size 1 (2 bytes) = header 0x09
        data = bytes([0x09, 0x00, 0x01])
        val = (data[1] << 8) | data[2]
        self.assertEqual(val, 1)

    def test_text_string(self):
        # Type 4 (text), size 5 (uint8 length) = header 0x25
        text = b"Apple Wireless Keyboard"
        data = bytes([0x25, len(text)]) + text
        result = data[2:2 + data[1]].decode()
        self.assertEqual(result, "Apple Wireless Keyboard")

    def test_boolean_true(self):
        # Type 5 (boolean), size 0 (1 byte) = header 0x28
        data = bytes([0x28, 0x01])
        self.assertTrue(bool(data[1]))

    def test_boolean_false(self):
        data = bytes([0x28, 0x00])
        self.assertFalse(bool(data[1]))


class TestInputReport(unittest.TestCase):
    """Test HID Input Report 0x13 decode."""

    def test_device_ready(self):
        data = bytes([0x13, 0x01])
        self.assertTrue(bool(data[1] & 0x01))
        self.assertFalse(bool(data[1] & 0x02))

    def test_connection_request(self):
        data = bytes([0x13, 0x02])
        self.assertFalse(bool(data[1] & 0x01))
        self.assertTrue(bool(data[1] & 0x02))

    def test_both(self):
        data = bytes([0x13, 0x03])
        self.assertTrue(bool(data[1] & 0x01))
        self.assertTrue(bool(data[1] & 0x02))

    def test_none(self):
        data = bytes([0x13, 0x00])
        self.assertFalse(bool(data[1] & 0x01))
        self.assertFalse(bool(data[1] & 0x02))


class TestChipIdentification(unittest.TestCase):
    """Test BCM chip identification by product ID."""

    def _identify(self, pid):
        chips = {
            (0x0220, 0x022C): "Broadcom BCM2042 (ARM7TDMI, BT 2.0+EDR)",
            (0x0255, 0x0257): "Broadcom BCM2042 (ARM7TDMI, BT 2.0+EDR)",
            (0x024F, 0x0250): "Broadcom BCM20733 (ARM Cortex-M3, BT 4.0 LE)",
            (0x0267, 0x026C): "Broadcom BCM20733 (ARM Cortex-M3, BT 4.0 LE)",
        }
        for (lo, hi), chip in chips.items():
            if lo <= pid <= hi:
                return chip
        return "Broadcom (unknown variant)"

    def test_a1314_iso(self):
        self.assertIn("BCM2042", self._identify(0x0256))

    def test_a1314_ansi(self):
        self.assertIn("BCM2042", self._identify(0x0255))

    def test_a1644(self):
        self.assertIn("BCM20733", self._identify(0x024F))

    def test_a2449(self):
        self.assertIn("BCM20733", self._identify(0x0267))

    def test_unknown(self):
        self.assertIn("unknown", self._identify(0x9999))


class TestModaliasParse(unittest.TestCase):
    """Test Modalias string parsing."""

    def _parse(self, modalias):
        if not modalias or not modalias.startswith("usb:"):
            return None
        p = modalias[4:]
        return {
            "vendor_id": f"0x{int(p[1:5], 16):04X}",
            "product_id": f"0x{int(p[6:10], 16):04X}",
            "fw_version": f"{int(p[11:15], 16) >> 8}.{int(p[11:15], 16) & 0xFF:02d}",
        }

    def test_apple_a1314(self):
        r = self._parse("usb:v05ACp0256d0050")
        self.assertEqual(r["vendor_id"], "0x05AC")
        self.assertEqual(r["product_id"], "0x0256")
        self.assertEqual(r["fw_version"], "0.80")

    def test_none(self):
        self.assertIsNone(self._parse(None))

    def test_non_usb(self):
        self.assertIsNone(self._parse("bluetooth:foo"))


class TestCoD(unittest.TestCase):
    """Test Bluetooth Class of Device decode."""

    def test_keyboard(self):
        cod = 0x2540
        major = (cod >> 8) & 0x1F
        minor = (cod >> 2) & 0x3F
        self.assertEqual(major, 5)  # Peripheral
        periph_sub = (minor >> 4) & 0x03
        self.assertEqual(periph_sub, 1)  # Keyboard


class TestRSSIQuality(unittest.TestCase):
    """Test RSSI quality classification."""

    def _quality(self, rssi):
        if rssi is None:
            return "N/A"
        if rssi >= -10:
            return "Optimal (golden range)"
        if rssi >= -65:
            return "Good"
        if rssi >= -80:
            return "Fair"
        return "Poor"

    def test_optimal(self):
        self.assertIn("Optimal", self._quality(0))
        self.assertIn("Optimal", self._quality(-5))

    def test_good(self):
        self.assertIn("Good", self._quality(-30))

    def test_fair(self):
        self.assertIn("Fair", self._quality(-70))

    def test_poor(self):
        self.assertIn("Poor", self._quality(-90))

    def test_none(self):
        self.assertEqual(self._quality(None), "N/A")


# ---------------------------------------------------------------------------
# Regression tests against the real scripts (loaded as modules, no hardware)
# ---------------------------------------------------------------------------

import argparse
import contextlib
import importlib.machinery
import importlib.util
import io
import os
import stat
import subprocess
import tempfile
from unittest import mock

_ROOT = Path(__file__).parent.parent


def _load_script(filename, modname):
    loader = importlib.machinery.SourceFileLoader(modname, str(_ROOT / filename))
    spec = importlib.util.spec_from_loader(modname, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


kb = _load_script("apple-kb-monitor", "apple_kb_monitor_under_test")


class TestHistoryRobustness(unittest.TestCase):
    """Corrupt / non-object history lines must never crash the readers."""

    LINES = [
        '{"t": "2026-09-01T10:00:00", "bat": 90, "fine": 91, "volt": 2.9, "rssi": 0}',
        "5", "[1, 2]", "null", "not json", '"str"',
        '{"t": null, "volt": "abc"}',
        '{"t": "2026-09-01T12:00:00", "bat": 88, "fine": 89, "volt": 2.8, "rssi": -40}',
    ]

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name) / "history.jsonl"
        self.path.write_text("\n".join(self.LINES) + "\n")

    def tearDown(self):
        self.tmp.cleanup()

    def test_read_history_keeps_only_objects(self):
        entries = kb.read_history(self.path)
        self.assertEqual(len(entries), 3)
        self.assertTrue(all(isinstance(e, dict) for e in entries))

    def test_read_history_missing_file(self):
        self.assertIsNone(kb.read_history(Path(self.tmp.name) / "nope"))

    def test_estimate_discharge_ignores_garbage(self):
        res = kb.estimate_discharge(self.path)
        self.assertIsNotNone(res)
        self.assertEqual(res["samples"], 2)
        self.assertGreater(res["rate_mv_per_hour"], 0)

    def test_graph_and_csv_do_not_raise(self):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            kb.print_graph(self.path)
            kb.export_csv(self.path)
        self.assertIn("timestamp,mac", out.getvalue())

    def test_csv_none_cells_are_empty(self):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            kb.export_csv(self.path)
        rows = out.getvalue().splitlines()[1:]
        self.assertEqual(len(rows), 3)
        self.assertTrue(all(r.count(",") == 7 for r in rows))

    def test_print_history_shows_zero_dbm(self):
        with mock.patch.object(kb, "HISTORY_FILE", self.path), \
                contextlib.redirect_stdout(io.StringIO()) as out:
            kb.print_history()
        self.assertIn("0dBm", out.getvalue())


class TestConnInfoRobustness(unittest.TestCase):
    def test_helper_not_executable_falls_back(self):
        with mock.patch.object(Path, "is_file", return_value=True), \
                mock.patch.object(kb.subprocess, "run", side_effect=PermissionError), \
                mock.patch.object(kb.socket, "socket", side_effect=PermissionError):
            self.assertIsNone(kb.get_conn_info("aa:bb:cc:dd:ee:ff"))

    def test_invalid_mac_returns_none(self):
        with mock.patch.object(Path, "is_file", return_value=False):
            self.assertIsNone(kb.get_conn_info("not-a-mac"))

    def test_socket_closed_on_timeout(self):
        sock = mock.MagicMock()
        sock.recv.side_effect = TimeoutError
        with mock.patch.object(Path, "is_file", return_value=False), \
                mock.patch.object(kb.socket, "socket", return_value=sock):
            self.assertIsNone(kb.get_conn_info("aa:bb:cc:dd:ee:ff"))
        sock.close.assert_called_once()

    def test_second_helper_is_tried(self):
        ok = mock.Mock(returncode=0, stdout='{"rssi": -3, "tx_power": 4, "max_tx_power": 8}')
        calls = []

        def fake_run(cmd, **kw):
            calls.append(cmd[0])
            if len(calls) == 1:
                raise PermissionError
            return ok

        with mock.patch.object(Path, "is_file", return_value=True), \
                mock.patch.object(kb.subprocess, "run", side_effect=fake_run):
            res = kb.get_conn_info("aa:bb:cc:dd:ee:ff")
        self.assertEqual(res["rssi_dbm"], -3)
        self.assertEqual(len(calls), 2)


class TestDumpPermission(unittest.TestCase):
    def test_dump_without_permission_returns_empty(self):
        with mock.patch.object(kb.os, "open", side_effect=PermissionError(13, "Permission denied")), \
                contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertEqual(kb.dump_all_reports("/dev/hidraw99"), [])
        self.assertIn("Cannot open", err.getvalue())


class TestSdpRobustness(unittest.TestCase):
    def test_truncated_uint8_does_not_raise(self):
        self.assertEqual(kb._read_sdp_element(bytes([0x08]), 0), (None, 0))

    def test_truncated_bool_does_not_raise(self):
        self.assertEqual(kb._read_sdp_element(bytes([0x28]), 0), (None, 0))

    def test_truncated_text_does_not_raise(self):
        self.assertEqual(kb._read_sdp_element(bytes([0x25, 0x10, 0x41]), 0), (None, 0))

    def test_nil_at_end_is_consumed(self):
        self.assertEqual(kb._read_sdp_element(bytes([0x00]), 0), (None, 1))

    def test_valid_uint16_still_decodes(self):
        self.assertEqual(kb._read_sdp_element(bytes([0x09, 0x12, 0x34]), 0), (0x1234, 3))

    def test_deep_nesting_is_bounded(self):
        inner = bytes([0x08, 0x01])
        for _ in range(200):
            inner = bytes([0x35, len(inner)]) + inner if len(inner) < 256 else inner
        # must terminate without RecursionError
        kb._read_sdp_element(inner, 0)
        nested = bytes([0x35, 0x02, 0x08, 0x01])
        for _ in range(40):
            nested = bytes([0x36, len(nested) >> 8, len(nested) & 0xFF]) + nested
        kb._read_sdp_element(nested, 0)


class TestJsonDefault(unittest.TestCase):
    def test_variant_and_bytes(self):
        if not kb.HAS_DBUS_FAST:
            self.skipTest("dbus-fast missing")
        data = {"ManufacturerData": {76: kb.Variant("ay", b"\x01\x02")}, "b": b"\xff"}
        out = json.loads(json.dumps(data, default=kb.json_default))
        self.assertEqual(out["b"], "ff")
        self.assertEqual(out["ManufacturerData"]["76"], "0102")


class TestBatteryProvider(unittest.TestCase):
    def setUp(self):
        if not kb.HAS_DBUS_FAST:
            self.skipTest("dbus-fast missing")

        class FakeBus:
            def __init__(self):
                self.paths = set()

            def export(self, path, iface):
                if path in self.paths:
                    raise ValueError(f"already exported: {path}")
                self.paths.add(path)

            def unexport(self, path, iface=None):
                self.paths.discard(path)

        self.bus = FakeBus()
        self.prov = kb.BatteryProvider(self.bus)

    def test_lowercase_add_uppercase_remove(self):
        self.prov.add_device("04:db:56:ca:42:ee", 80)
        self.assertTrue(self.prov.has_device("04:DB:56:CA:42:EE"))
        self.prov.remove_device("04:DB:56:CA:42:EE")
        self.assertFalse(self.prov.has_device("04:db:56:ca:42:ee"))
        self.assertEqual(self.bus.paths, {kb.PROVIDER_ROOT})

    def test_add_twice_updates_instead_of_double_export(self):
        self.prov.add_device("04:db:56:ca:42:ee", 80)
        self.prov.add_device("04:DB:56:CA:42:EE", 70)
        self.assertEqual(len(self.prov.batteries), 1)

    def test_reset_unexports_so_device_can_be_re_added(self):
        self.prov.add_device("04:db:56:ca:42:ee", 80)
        self.prov.reset()
        self.assertEqual(self.prov.batteries, {})
        self.assertEqual(self.bus.paths, {kb.PROVIDER_ROOT})
        self.prov.add_device("04:db:56:ca:42:ee", 75)  # would raise before the fix
        self.assertEqual(len(self.prov.batteries), 1)

    def test_empty_mac_ignored(self):
        self.prov.add_device("", 50)
        self.assertEqual(self.prov.batteries, {})

    def test_setup_provider_reuses_existing_provider(self):
        import asyncio

        self.prov.add_device("04:db:56:ca:42:ee", 80)

        async def fake_call(msg):
            return mock.Mock(message_type=None)

        self.bus.call = fake_call
        with mock.patch.object(kb, "find_devices", return_value=[]):
            res = asyncio.run(kb._setup_provider(self.bus, self.prov))
        self.assertIs(res, self.prov)
        self.assertEqual(self.prov.batteries, {})


class TestFindDevices(unittest.TestCase):
    def test_malformed_hid_id_is_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            good = Path(tmp) / "hidraw0" / "device"
            bad = Path(tmp) / "hidraw1" / "device"
            for d, hid in ((good, "0005:000005AC:00000256"), (bad, "0005:ZZZZ:QQQQ")):
                d.mkdir(parents=True)
                (d / "uevent").write_text(f"HID_ID={hid}\nHID_UNIQ=aa:bb:cc:dd:ee:ff\n")
            paths = sorted(str(Path(tmp) / f"hidraw{i}") for i in (0, 1))
            real_glob = kb.glob.glob
            with mock.patch.object(
                kb.glob, "glob",
                side_effect=lambda pat: paths if "hidraw*" in pat else real_glob(pat),
            ):
                devs = kb.find_devices()
        self.assertEqual(len(devs), 1)
        self.assertEqual(devs[0]["pid"], 0x0256)


class TestStateDir(unittest.TestCase):
    def test_created_private(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "state"
            with mock.patch.object(kb, "STATE_DIR", target):
                kb.ensure_state_dir()
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o700)

    def test_foreign_owner_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "state"
            with mock.patch.object(kb, "STATE_DIR", target):
                kb.ensure_state_dir()
                with mock.patch.object(kb.os, "getuid", return_value=os.getuid() + 1):
                    with self.assertRaises(PermissionError):
                        kb.ensure_state_dir()

    def test_fallback_name_has_uid(self):
        env = {k: v for k, v in os.environ.items() if k != "XDG_RUNTIME_DIR"}
        code = ("import importlib.machinery as m, importlib.util as u;"
                f"l=m.SourceFileLoader('x', {str(_ROOT / 'apple-kb-monitor')!r});"
                "s=u.spec_from_loader('x',l);mod=u.module_from_spec(s);l.exec_module(mod);"
                "print(mod.STATE_DIR)")
        out = subprocess.run([sys.executable, "-c", code], env=env,
                             capture_output=True, text=True, timeout=30).stdout.strip()
        self.assertEqual(out, f"/tmp/apple-kb-monitor-{os.getuid()}")


class TestCliTypes(unittest.TestCase):
    def test_positive_int(self):
        self.assertEqual(kb.positive_int("5"), 5)
        for bad in ("0", "-3", "x"):
            with self.assertRaises(argparse.ArgumentTypeError):
                kb.positive_int(bad)

    def test_percent_int(self):
        self.assertEqual(kb.percent_int("15"), 15)
        for bad in ("101", "-1", "x"):
            with self.assertRaises(argparse.ArgumentTypeError):
                kb.percent_int(bad)

    def test_interval_zero_rejected_by_cli(self):
        r = subprocess.run([sys.executable, str(_ROOT / "apple-kb-monitor"), "--interval", "0"],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 2)
        self.assertIn("must be >= 1", r.stderr)

    def test_led_invalid_state_rejected(self):
        r = subprocess.run([sys.executable, str(_ROOT / "apple-kb-monitor"),
                            "--led", "capslock", "maybe"],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 2)


class TestMqttClientFactory(unittest.TestCase):
    def test_paho2_uses_callback_api_version(self):
        mod = mock.Mock()
        mod.CallbackAPIVersion.VERSION1 = "V1"
        kb.new_mqtt_client(mod, "cid")
        mod.Client.assert_called_once_with("V1", client_id="cid")

    def test_paho1_plain_client(self):
        class Mod:
            Client = mock.Mock()

        kb.new_mqtt_client(Mod, "cid")
        Mod.Client.assert_called_once_with(client_id="cid")

    def test_mosquitto_pub_timeout_is_handled(self):
        dev = {"mac": "aa:bb:cc:dd:ee:ff", "path": "/dev/null"}
        with mock.patch.object(kb, "read_all_reports", return_value={}), \
                mock.patch.object(kb, "get_conn_info", return_value=None), \
                mock.patch.object(kb.subprocess, "run",
                                  side_effect=subprocess.TimeoutExpired("x", 5)), \
                contextlib.redirect_stderr(io.StringIO()):
            kb._mqtt_publish_cli("127.0.0.1", 1883, "t", [dev])  # must not raise


class TestPollDevice(unittest.TestCase):
    def test_no_reading_is_a_noop(self):
        dev = {"mac": "aa:bb:cc:dd:ee:ff", "path": "/dev/null", "name": "kb"}
        with mock.patch.object(kb, "read_all_reports", return_value={}), \
                mock.patch.object(kb, "append_history") as hist:
            kb._poll_device(dev, None, 15)
        hist.assert_not_called()

    def test_low_battery_notifies_once(self):
        dev = {"mac": "aa:bb:cc:dd:ee:ff", "path": "/dev/null", "name": "kb"}
        with tempfile.TemporaryDirectory() as tmp:
            flag = Path(tmp) / "low_notified"
            with mock.patch.object(kb, "read_all_reports",
                                   return_value={"battery_pct": 10, "voltage": 2.2}), \
                    mock.patch.object(kb, "get_conn_info", return_value=None), \
                    mock.patch.object(kb, "append_history"), \
                    mock.patch.object(kb, "NOTIFIED_FILE", flag), \
                    mock.patch.object(kb, "notify") as notify:
                kb._poll_device(dev, None, 15)
                kb._poll_device(dev, None, 15)
            self.assertEqual(notify.call_count, 1)


class TestAutoBrightness(unittest.TestCase):
    def test_missing_ddc_tool_is_clean_error(self):
        with mock.patch.object(sys, "argv", ["apple-kb-monitor", "--auto-brightness"]), \
                mock.patch.object(kb.subprocess, "run", side_effect=FileNotFoundError), \
                contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as cm:
                kb.main()
        self.assertEqual(cm.exception.code, 1)


class TestMqttBridge(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        try:
            cls.br = _load_script("mqtt-bridge.py", "mqtt_bridge_under_test")
        except ImportError:
            raise unittest.SkipTest("paho-mqtt missing")

    def test_invalid_toml_exits_cleanly(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "config.toml"
            bad.write_text("[mqtt\nbroker = = 1")
            with mock.patch.object(self.br, "CONFIG_PATHS", [bad]), \
                    contextlib.redirect_stderr(io.StringIO()) as err:
                with self.assertRaises(SystemExit) as cm:
                    self.br.load_config()
        self.assertEqual(cm.exception.code, 1)
        self.assertIn("cannot read", err.getvalue())

    def test_inf_payload_does_not_raise(self):
        msg = mock.Mock(payload=b"inf")
        client = mock.Mock()
        ud = {"bri_min": 2, "bri_max": 70, "bus": "6", "topic_state": "s"}
        with mock.patch.object(self.br, "ddc_write_brightness") as w, \
                contextlib.redirect_stderr(io.StringIO()):
            self.br.on_message(client, ud, msg)
        w.assert_not_called()

    def test_payload_is_clamped(self):
        msg = mock.Mock(payload=b"500")
        client = mock.Mock()
        ud = {"bri_min": 2, "bri_max": 70, "bus": "6", "topic_state": "s"}
        with mock.patch.object(self.br, "ddc_write_brightness", return_value=True) as w:
            self.br.on_message(client, ud, msg)
        self.assertEqual(w.call_args[0][3], 70)
        client.publish.assert_called_once_with("s", "70", retain=True)

    def test_ddc_timeout_is_handled(self):
        with mock.patch.object(self.br.subprocess, "run",
                               side_effect=subprocess.TimeoutExpired("x", 5)), \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertFalse(self.br.ddc_write_brightness("6", 2, 70, 30))
            self.assertEqual(self.br.ddc_read_brightness("6"), -1)

    def test_paho2_client_factory(self):
        fake = mock.Mock()
        fake.CallbackAPIVersion.VERSION1 = "V1"
        with mock.patch.object(self.br, "mqtt", fake):
            self.br.new_mqtt_client("cid", {"a": 1})
        fake.Client.assert_called_once_with("V1", client_id="cid", userdata={"a": 1})


class TestApihubSettings(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        try:
            from PySide6.QtWidgets import QApplication
        except ImportError:
            raise unittest.SkipTest("PySide6 missing")
        cls.app = QApplication.instance() or QApplication([])
        cls.ah = _load_script("apihub-settings", "apihub_settings_under_test")

    def test_as_int_and_txt(self):
        self.assertEqual(self.ah._as_int(None), 0)
        self.assertEqual(self.ah._as_int("12"), 12)
        self.assertEqual(self.ah._as_int(3.9), 3)
        self.assertEqual(self.ah._txt(None), "--")
        self.assertEqual(self.ah._txt(""), "--")
        self.assertEqual(self.ah._txt(0), "0")

    def test_update_survives_null_values(self):
        with mock.patch.object(self.ah, "ddc_write"):
            win = self.ah.ApiHubWindow()
            kb_json = {
                "battery": {"percentage": None, "voltage": None},
                "radio": {}, "device": {"model": None, "mac": None, "driver": None},
                "firmware": {"version": None}, "bluetooth": {"interval": None},
                "analysis": {"battery_type": {}, "discharge": None},
            }
            mon = {"brightness": {"current": None}, "firmware": "x", "volume": {}}
            win._update(mon, kb_json)
            self.assertIn("Keyboard: OK", win.statusBar().currentMessage())
            win._timer.stop()

    def test_ddc_write_swallows_missing_binary(self):
        import threading
        done = threading.Event()

        def boom(*a, **k):
            done.set()
            raise FileNotFoundError

        with mock.patch.object(self.ah.subprocess, "run", side_effect=boom), \
                contextlib.redirect_stderr(io.StringIO()) as err:
            self.ah.ddc_write(16, 10)
            self.assertTrue(done.wait(5))
            time_left = 50
            while "failed" not in err.getvalue() and time_left:
                threading.Event().wait(0.02)
                time_left -= 1
        self.assertIn("failed", err.getvalue())

    def test_slider_drag_is_debounced(self):
        with mock.patch.object(self.ah, "ddc_write") as w:
            s = self.ah.ValueSlider("Brightness", 16)
            for v in (10, 20, 30, 40):
                s.slider.setValue(v)
            w.assert_not_called()
            s._debounce.stop()
            s._flush()
            w.assert_called_once_with(16, 40)


class TestAuditFixes(unittest.TestCase):
    FIX = Path(__file__).parent / "fixtures" / "a1314_iso"
    MAC = "04:db:56:ca:42:ee"

    def test_ps_path_matches_suffix(self):  # #70
        name = (self.FIX / "power_supply.uevent").read_text()
        ps_name = [l.split("=", 1)[1] for l in name.splitlines()
                   if l.startswith("POWER_SUPPLY_NAME=")][0]
        self.assertEqual(ps_name, f"hid-{self.MAC}-battery-71")
        with mock.patch.object(kb.glob, "glob",
                               side_effect=lambda pat: [f"/sys/class/power_supply/{ps_name}"]
                               if "-battery*" in pat else []):
            self.assertEqual(kb.find_power_supply(self.MAC),
                             f"/sys/class/power_supply/{ps_name}")
        self.assertIsNone(kb.find_power_supply(""))

    def test_sysfs_fallback_when_hid_unreadable(self):  # #71
        import asyncio
        with tempfile.TemporaryDirectory() as tmp:
            for f in ("capacity", "status"):
                (Path(tmp) / f).write_text((self.FIX / f"ps_{f}").read_text())
            dev = {"path": "/dev/null", "mac": "", "ps_path": tmp, "model": "m",
                   "name": "kb", "chip": "c", "driver": "d"}
            with mock.patch.object(kb, "read_all_reports", return_value={}), \
                    mock.patch.object(kb, "get_conn_info", return_value=None), \
                    mock.patch.object(kb, "read_input_caps", return_value={}), \
                    mock.patch.object(kb, "read_leds", return_value={}), \
                    contextlib.redirect_stderr(io.StringIO()):
                rep = asyncio.run(kb.collect_report(None, dev))
        self.assertEqual(rep["battery"]["percentage"], 90)
        self.assertEqual(rep["battery"]["percentage_source"], "sysfs")

    def test_no_zero_percent_published_without_reading(self):  # #78
        import asyncio
        if not kb.HAS_DBUS_FAST:
            self.skipTest("dbus-fast missing")
        prov = mock.MagicMock()

        async def ok():
            return True

        prov.register.return_value = ok()
        dev = {"mac": "AA:BB:CC:DD:EE:FF", "path": "/dev/null"}
        with mock.patch.object(kb, "find_devices", return_value=[dev]), \
                mock.patch.object(kb, "read_all_reports", return_value={}), \
                contextlib.redirect_stderr(io.StringIO()):
            asyncio.run(kb._setup_provider(mock.MagicMock(), prov))
        prov.add_device.assert_not_called()
        self.assertEqual(kb.best_level({"battery_pct": 0}), 0)
        self.assertIsNone(kb.best_level({}))

    def _mgmt_sock(self, packets):
        sock = mock.MagicMock()
        sock.recv.side_effect = packets
        return sock

    @staticmethod
    def _evt(ev, op, status, rssi=-40, tx=4, mx=8):
        return struct.pack("<HHHHB", ev, 0, 10, op, status) + b"\0" * 7 \
            + struct.pack("<bbb", rssi, tx, mx)

    def test_mgmt_rejects_127(self):  # #77
        sock = self._mgmt_sock([self._evt(1, 0x31, 0, rssi=127)])
        with mock.patch.object(Path, "is_file", return_value=False), \
                mock.patch.object(kb.socket, "socket", return_value=sock):
            self.assertIsNone(kb.get_conn_info("aa:bb:cc:dd:ee:ff"))

    def test_mgmt_skips_foreign_event(self):  # #77
        foreign = self._evt(1, 0x0004, 0, rssi=-1)
        sock = self._mgmt_sock([foreign, self._evt(1, 0x31, 0, rssi=-40)])
        with mock.patch.object(Path, "is_file", return_value=False), \
                mock.patch.object(kb.socket, "socket", return_value=sock):
            res = kb.get_conn_info("aa:bb:cc:dd:ee:ff")
        self.assertEqual(res["rssi_dbm"], -40)

    def test_calibration_validation(self):  # #80
        self.assertTrue(kb.calibration_valid([2900, 2450, 2350, 2000]))
        self.assertFalse(kb.calibration_valid([0, 0, 0, 0]))
        self.assertFalse(kb.calibration_valid([2000, 2450, 2350, 2900]))
        self.assertFalse(kb.calibration_valid([2900, 2900, 2350, 2000]))
        bad = bytes([kb.REP_CALIB_CURVE, 0, 0, 0, 0, 0, 0, 0, 0])

        def fake(devpath, rid, *a, **k):
            if rid == kb.REP_CALIB_CURVE:
                return bad
            return None
        with mock.patch.object(kb, "hid_get_feature", side_effect=fake):
            r = kb.read_all_reports("/dev/null")
        self.assertFalse(r["calib_valid"])


if __name__ == "__main__":
    unittest.main()
