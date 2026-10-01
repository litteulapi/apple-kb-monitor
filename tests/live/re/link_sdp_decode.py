#!/usr/bin/env python3
"""Décodeur hors ligne des enregistrements SDP mis en cache par BlueZ (lecture seule).

Source : /var/lib/bluetooth/<adaptateur>/cache/<appareil>, section [ServiceRecords]
(lecture root : `sudo -n cat …`, aucune clé dans ce fichier). Aucune requête radio.

Usage :
    link_sdp_decode.py tests/fixtures/a1314_iso/link_sdp_cache.txt [--json]
"""
from __future__ import annotations

import json
import sys

ATTR = {
    0x0000: "ServiceRecordHandle", 0x0001: "ServiceClassIDList", 0x0002: "ServiceRecordState",
    0x0003: "ServiceID", 0x0004: "ProtocolDescriptorList", 0x0005: "BrowseGroupList",
    0x0006: "LanguageBaseAttributeIDList", 0x0007: "ServiceInfoTimeToLive",
    0x0008: "ServiceAvailability", 0x0009: "BluetoothProfileDescriptorList",
    0x000A: "DocumentationURL", 0x000B: "ClientExecutableURL", 0x000C: "IconURL",
    0x000D: "AdditionalProtocolDescriptorLists",
    0x0100: "ServiceName", 0x0101: "ServiceDescription", 0x0102: "ProviderName",
}
HID_ATTR = {
    0x0200: "HIDDeviceReleaseNumber (obsolète)", 0x0201: "HIDParserVersion",
    0x0202: "HIDDeviceSubclass", 0x0203: "HIDCountryCode", 0x0204: "HIDVirtualCable",
    0x0205: "HIDReconnectInitiate", 0x0206: "HIDDescriptorList", 0x0207: "HIDLANGIDBaseList",
    0x0208: "HIDSDPDisable (obsolète)", 0x0209: "HIDBatteryPower", 0x020A: "HIDRemoteWake",
    0x020B: "HIDProfileVersion (obsolète)", 0x020C: "HIDSupervisionTimeout",
    0x020D: "HIDNormallyConnectable", 0x020E: "HIDBootDevice", 0x020F: "HIDSSRHostMaxLatency",
    0x0210: "HIDSSRHostMinTimeout",
}
PNP_ATTR = {
    0x0200: "SpecificationID", 0x0201: "VendorID", 0x0202: "ProductID", 0x0203: "Version",
    0x0204: "PrimaryRecord", 0x0205: "VendorIDSource",
}
UUID16 = {
    0x0001: "SDP", 0x0011: "HIDP", 0x0100: "L2CAP", 0x1002: "PublicBrowseRoot",
    0x1124: "HumanInterfaceDeviceService", 0x1200: "PnPInformation",
}


def de(buf: bytes, i: int):
    """Décode un élément de données SDP ; renvoie (valeur, i_suivant)."""
    h = buf[i]; i += 1
    typ, sz = h >> 3, h & 7
    if typ == 0:
        return None, i
    if sz < 5:
        n = (1, 2, 4, 8, 16)[sz]
    else:
        ln = (1, 2, 4)[sz - 5]
        n = int.from_bytes(buf[i:i + ln], "big"); i += ln
    raw = buf[i:i + n]; i += n
    if typ == 1:
        return ("uint", int.from_bytes(raw, "big"), n), i
    if typ == 2:
        return ("int", int.from_bytes(raw, "big", signed=True), n), i
    if typ == 3:
        v = int.from_bytes(raw, "big")
        return ("uuid", v, n), i
    if typ == 4:
        return ("str", raw), i
    if typ == 5:
        return ("bool", bool(raw[0])), i
    if typ in (6, 7):
        out, j = [], 0
        while j < len(raw):
            v, j = de(raw, j)
            out.append(v)
        return ("seq" if typ == 6 else "alt", out), i
    if typ == 8:
        return ("url", raw), i
    return ("?", raw), i


def fmt(v, depth=0) -> str:
    k = v[0]
    if k == "uint":
        return f"0x{v[1]:0{2 * v[2]}X} ({v[1]})"
    if k == "uuid":
        name = UUID16.get(v[1], "")
        return f"UUID 0x{v[1]:0{2 * v[2]}X}{' ' + name if name else ''}"
    if k == "str":
        try:
            s = v[1].decode("utf-8")
            if s.isprintable():
                return repr(s)
        except UnicodeDecodeError:
            pass
        return f"octets[{len(v[1])}] {v[1].hex()}"
    if k == "bool":
        return str(v[1]).lower()
    if k in ("seq", "alt"):
        return "[" + ", ".join(fmt(x, depth + 1) for x in v[1]) + "]"
    return repr(v)


def decode_record(hexstr: str):
    buf = bytes.fromhex(hexstr)
    top, _ = de(buf, 0)
    items = top[1]
    attrs = {}
    for a, val in zip(items[0::2], items[1::2]):
        attrs[a[1]] = val
    return attrs


def main() -> int:
    path = sys.argv[1]
    as_json = "--json" in sys.argv
    recs = {}
    sect = None
    with open(path, encoding="utf-8") as f:
        for ln in f:
            ln = ln.strip()
            if ln.startswith("["):
                sect = ln
            elif sect == "[ServiceRecords]" and "=" in ln:
                h, v = ln.split("=", 1)
                recs[h] = decode_record(v)
    out = {}
    for h, attrs in recs.items():
        classes = attrs.get(0x0001, ("seq", []))[1]
        cls = classes[0][1] if classes else 0
        table = HID_ATTR if cls == 0x1124 else PNP_ATTR if cls == 0x1200 else {}
        rec = {}
        for aid in sorted(attrs):
            name = ATTR.get(aid) or table.get(aid) or ("LangBase+" + hex(aid - 0x100) if 0x100 <= aid < 0x200 else "?")
            val = attrs[aid]
            if aid == 0x0206:  # descripteur HID : extraire les octets
                desc = val[1][0][1][1][1]
                rec[f"0x{aid:04X} {name}"] = f"type 0x22, {len(desc)} octets, {desc.hex()}"
            else:
                rec[f"0x{aid:04X} {name}"] = fmt(val)
        out[h] = rec
    if as_json:
        print(json.dumps(out, ensure_ascii=False, indent=1))
    else:
        for h, rec in out.items():
            print(f"== enregistrement {h}")
            for k, v in rec.items():
                print(f"  {k} = {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
