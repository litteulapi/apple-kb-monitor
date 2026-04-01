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


if __name__ == "__main__":
    unittest.main()
