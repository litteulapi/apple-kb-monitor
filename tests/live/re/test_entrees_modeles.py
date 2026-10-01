#!/usr/bin/env python3
"""Entrées clavier et couverture des modèles, SANS matériel.

Vérifie à partir de fixtures (docs/HARDWARE-ENTREES-MODELES.md) :
- ce que chaque famille déclare côté entrées (descripteurs HID de référence) ;
- que la table des modèles Rust (akm-core/src/model.rs, lue en texte, non
  modifiée) correspond à hid-ids.h et classe chaque cas comme attendu ;
- la couverture de la configuration livrée (udev, keyd) et du code Python hérité.

Les défauts connus sont marqués expectedFailure avec le numéro d'issue Gitea :
le test passera (XPASS) quand le défaut sera corrigé.
Lancement : python3 -m pytest tests/live/re -q   (ou python3 <ce fichier>)
"""
import fnmatch
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FIX = ROOT / "tests" / "fixtures"
MODELS = FIX / "models"
sys.path.insert(0, str(HERE))
import hid_rdesc  # noqa: E402

APPLE_USB_VID, APPLE_BT_VID = 0x05AC, 0x004C


def rust_model_table():
    """{pid: family} parsed from akm-core/src/model.rs (APPLE_MODELS)."""
    src = (ROOT / "apihub-app" / "akm-core" / "src" / "model.rs").read_text()
    table = src.split("pub const APPLE_MODELS", 1)[1].split("];", 1)[0]
    out = {}
    for pid, fam in re.findall(r"m\(\s*0x([0-9a-fA-F]{4}),.*?Family::(\w+)", table, re.S):
        out[int(pid, 16)] = fam
    return out


def rust_lookup(table, vid, pid):
    """Mirror of model::lookup_model + family (both vendors accepted)."""
    if vid not in (APPLE_USB_VID, APPLE_BT_VID):
        return None
    return table.get(pid)


def parse_hid_id(hid_id):
    _bus, vid, pid = hid_id.split(":")
    return int(vid, 16), int(pid, 16)


def rust_is_keyboard_modalias(table, m):
    """Mirror of model::is_keyboard_device without Class (the watcher's filter, #124):
    modalias vendor:product looked up in the model table."""
    mm = re.match(r"^[a-z]+:v([0-9a-fA-F]{4})p([0-9a-fA-F]{4})", m or "")
    if not mm:
        return False
    return rust_lookup(table, int(mm.group(1), 16), int(mm.group(2), 16)) is not None


def kernel_keyboard_pids():
    """Wireless / Magic keyboard PIDs from the hid-ids.h snapshot."""
    pids = {}
    for line in (MODELS / "hid-ids-apple.txt").read_text().splitlines():
        m = re.search(r"USB_DEVICE_ID_APPLE_((ALU_WIRELESS|MAGIC_KEYBOARD)\w*)\s+0x([0-9a-fA-F]{4})", line)
        if m:
            pids[int(m.group(3), 16)] = m.group(1)
    return pids


def cases():
    return json.loads((MODELS / "families.json").read_text())["cases"]


def rdesc_fields(rel):
    return hid_rdesc.parse(hid_rdesc.load(MODELS / rel))


def kernel_name(hid_id, seq=1):
    """sysfs HID device name, as matched by udev KERNELS: 0005:05AC:0256.0016."""
    bus, vid, pid = hid_id.split(":")
    return f"{bus}:{vid[-4:].upper()}:{pid[-4:].upper()}.{seq:04X}"


class TestDescripteurA1314(unittest.TestCase):
    """Descripteur capturé sur l'A1314 ISO réel (224 octets)."""

    @classmethod
    def setUpClass(cls):
        cls.f = hid_rdesc.parse(hid_rdesc.load(FIX / "a1314_iso" / "report_descriptor.hex"))

    def test_identique_au_selftest_noyau(self):
        real = hid_rdesc.load(FIX / "a1314_iso" / "report_descriptor.hex")
        ref = hid_rdesc.load(MODELS / "kernel_selftest_0005_05ac_0256.hex")
        self.assertEqual(real, ref)

    def test_rapports_declares(self):
        self.assertEqual(hid_rdesc.reports(self.f), {
            ("input", 0x01): 64, ("output", 0x01): 8, ("input", 0x47): 8,
            ("input", 0x11): 8, ("input", 0x12): 8, ("input", 0x13): 8,
            ("feature", 0x09): 24,
        })

    def test_0x11_ejection_et_fn(self):
        bits = hid_rdesc.usage_bits(self.f, "input", 0x11)
        self.assertEqual(bits, {0x000C00B8: 3, 0x00FF0003: 4})

    def test_0x12_medias(self):
        bits = hid_rdesc.usage_bits(self.f, "input", 0x12)
        self.assertEqual(bits, {0x000C00CD: 0, 0x000C00B3: 1, 0x000C00B4: 2,
                                0x000C00B5: 3, 0x000C00B6: 4})

    def test_0x13_reveil(self):
        bits = hid_rdesc.usage_bits(self.f, "input", 0x13)
        self.assertEqual(bits, {0xFF01000A: 0, 0xFF01000C: 1})

    def test_0x47_batterie_en_entree(self):
        bat = [x for x in self.f if x.report_id == 0x47 and not x.constant]
        self.assertEqual([(x.kind, x.size, x.count, x.usages) for x in bat],
                         [("input", 8, 1, [0x00060020])])  # Battery Strength, 0..255

    def test_sortie_led_5_usages(self):
        out = [x for x in self.f if x.kind == "output" and not x.constant]
        self.assertEqual(out[0].usages, [0x00080001 + i for i in range(5)])

    def test_evtest_capacites(self):
        cap = (FIX / "a1314_iso" / "evtest_capabilities.txt").read_text()
        for key in ("KEY_FN", "KEY_EJECTCD", "KEY_SCALE", "KEY_DASHBOARD", "KEY_NUMLOCK",
                    "KEY_PLAYPAUSE", "KEY_BRIGHTNESSDOWN", "KEY_KBDILLUMUP"):
            self.assertIn(f"({key})", cap)
        for led in ("LED_NUML", "LED_CAPSL", "LED_SCROLLL", "LED_COMPOSE", "LED_KANA"):
            self.assertIn(f"({led})", cap)


class TestDescripteurMagicKeyboard2021(unittest.TestCase):
    """Descripteur publié (gist xloc, dump ESP32) du 004C:029C en BT."""

    @classmethod
    def setUpClass(cls):
        cls.f = rdesc_fields("mk2021_0005_004c_029c_bt.hex")

    def test_rapports(self):
        self.assertEqual(set(hid_rdesc.reports(self.f)), {
            ("input", 0x01), ("output", 0x01), ("feature", 0x55), ("input", 0x90)})

    def test_fn_et_verrou_dans_le_rapport_clavier(self):
        bits = hid_rdesc.usage_bits(self.f, "input", 0x01)
        self.assertIn(0xFF010003, bits)  # HID_UP_HPVENDOR2|3 -> KEY_FN (hid-apple)
        self.assertIn(0x000C019E, bits)  # AL Terminal Lock -> KEY_COFFEE/SCREENLOCK

    def test_batterie_power_page(self):
        bits = hid_rdesc.usage_bits(self.f, "input", 0x90)
        self.assertIn(0x00850044, bits)  # Charging
        self.assertTrue(hid_rdesc.has_usage(self.f, 0x00850065, "input"))  # AbsoluteStateOfCharge

    def test_pas_de_rapports_bcm2042(self):
        ids = {rid for (_k, rid) in hid_rdesc.reports(self.f)}
        self.assertFalse(ids & {0x11, 0x12, 0x13, 0x47})


class TestTableDesModeles(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.table = rust_model_table()

    def test_17_pid_egaux_a_hid_ids(self):
        self.assertEqual(len(self.table), 17)
        self.assertEqual(set(self.table), set(kernel_keyboard_pids()))

    def test_famille_coherente_avec_le_nom_noyau(self):
        for pid, name in kernel_keyboard_pids().items():
            want = "Bcm2042" if name.startswith("ALU_WIRELESS") else "MagicKeyboard"
            self.assertEqual(self.table[pid], want, f"{pid:#06x} {name}")

    def test_detection_par_cas(self):
        for c in cases():
            if not c["hid_id"]:
                continue
            with self.subTest(c["name"]):
                vid, pid = parse_hid_id(c["hid_id"])
                self.assertEqual(rust_lookup(self.table, vid, pid), c["expected_family"])

    def test_rapport_0x13_seulement_bcm2042(self):
        """Le moniteur de réveil n'a de sens que si le descripteur déclare 0x13 (#129)."""
        for c in cases():
            if not c["rdesc"]:
                continue
            f = rdesc_fields(c["rdesc"])
            has13 = ("input", 0x13) in hid_rdesc.reports(f)
            self.assertEqual(has13, c["expected_family"] == "Bcm2042", c["name"])

    def test_watcher_modalias_filter_rejects_non_keyboards(self):  # #124
        src = (ROOT / "apihub-app" / "apple-kb-monitord" / "src" / "watcher.rs").read_text()
        self.assertIn("is_keyboard_device", src)
        self.assertNotIn("is_apple_modalias", src)
        neg = [c["name"] for c in cases() if c["modalias"] and c["expected_family"] is None
               and rust_is_keyboard_modalias(self.table, c["modalias"])]
        self.assertEqual(neg, [])  # AirPods, Magic Mouse/Trackpad 1 & 2 rejected
        pos = [c["name"] for c in cases() if c["modalias"] and c["expected_family"]
               and not rust_is_keyboard_modalias(self.table, c["modalias"])]
        self.assertEqual(pos, [])


class TestConfigurationLivree(unittest.TestCase):
    def test_udev_uaccess_couvre_les_claviers(self):
        rule = (ROOT / "udev" / "70-apple-kb-hidraw.rules").read_text()
        pats = re.search(r'KERNELS=="([^"]+)"', rule).group(1).split("|")
        for c in cases():
            if c["hid_id"] and c["expected_family"]:
                with self.subTest(c["name"]):
                    name = kernel_name(c["hid_id"])
                    self.assertTrue(any(fnmatch.fnmatchcase(name, p) for p in pats), name)

    def test_udev_trop_large_documente(self):
        """Constat (doc §6) : la règle donne aussi uaccess aux souris/trackpads Apple."""
        rule = (ROOT / "udev" / "70-apple-kb-hidraw.rules").read_text()
        pats = re.search(r'KERNELS=="([^"]+)"', rule).group(1).split("|")
        mouse = kernel_name("0005:0000004C:00000269")
        self.assertTrue(any(fnmatch.fnmatchcase(mouse, p) for p in pats))

    def test_keyd_ids_cover_supported_models(self):  # #126
        conf = (ROOT / "keyd" / "apple-keyboard.conf").read_text()
        ids = conf.split("[ids]", 1)[1].split("[", 1)[0].split()
        for c in cases():
            if c["hid_id"] and c["expected_family"] and c["hid_id"].startswith("0005"):
                vid, pid = parse_hid_id(c["hid_id"])
                self.assertIn(f"{vid:04x}:{pid:04x}", ids, c["name"])

    def test_keyd_f3_f6_symetriques_toutes_tables(self):  # #126
        conf = (ROOT / "keyd" / "apple-keyboard.conf").read_text()
        main = conf.split("[main]", 1)[1]
        binds = dict(
            (k.strip(), v.strip()) for k, v in
            (l.split("=", 1) for l in main.splitlines() if "=" in l and not l.lstrip().startswith("#")))
        groups = {"M-z": ("f3", "scale"),
                  "M-g": ("f4", "dashboard", "search"),
                  "M-l": ("f5", "kbdillumdown", "micmute"),
                  "M-d": ("f6", "numlock", "kbdillumup", "sleep")}
        for macro, keys in groups.items():
            for k in keys:
                self.assertEqual(binds.get(k), f"macro({macro})", k)


def load_python_legacy():
    src = ROOT / "apple-kb-monitor"
    g = {"__file__": str(src), "__name__": "apple_kb_monitor"}
    code = src.read_text().split("\nif __name__")[0]
    exec(compile(code, str(src), "exec"), g)  # noqa: S102 - project script
    return g


class TestPythonHerite(unittest.TestCase):
    @unittest.expectedFailure  # #127
    def test_python_legacy_table_matches_kernel(self):
        g = load_python_legacy()
        self.assertEqual(set(g["APPLE_PRODUCTS"]), set(kernel_keyboard_pids()))

    @unittest.expectedFailure  # #127
    def test_python_find_devices_rejects_mouse_and_sees_magic_keyboard(self):
        g = load_python_legacy()
        with tempfile.TemporaryDirectory() as tmp:
            sysroot = Path(tmp)
            entries = [("hidraw0", "0005:000005AC:0000030D", "aa:bb:cc:dd:ee:01"),   # Magic Mouse 1
                       ("hidraw1", "0005:0000004C:0000029C", "aa:bb:cc:dd:ee:02")]   # Magic Keyboard 2021
            for node, hid, mac in entries:
                d = sysroot / "class" / "hidraw" / node / "device"
                d.mkdir(parents=True)
                (d / "uevent").write_text(f"HID_ID={hid}\nHID_NAME=x\nHID_UNIQ={mac}\n")
            real_glob = g["glob"].glob

            class Shim:
                @staticmethod
                def glob(p):
                    if p.startswith("/sys/"):
                        p = str(sysroot) + p[4:]
                    return real_glob(p)

                escape = staticmethod(g["glob"].escape)
            g["glob"] = Shim
            found = {d["pid"] for d in g["find_devices"]()}
        self.assertEqual(found, {0x029C})


if __name__ == "__main__":
    unittest.main()
