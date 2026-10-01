#!/usr/bin/env python3
"""Replay recorded frames through the Python decoder of apple-kb-monitor
(read_all_reports) without touching the keyboard: hid_get_feature is
replaced by a lookup that mimics HIDIOCGFEATURE(16) (payload copied,
rest of the 16-byte buffer left at zero).

usage: replay_python_decode.py frames.hex   (one frame per line, hex)
"""
import importlib.machinery, importlib.util, json, sys
from pathlib import Path

script = Path(__file__).resolve().parents[3] / "apple-kb-monitor"
loader = importlib.machinery.SourceFileLoader("akm", str(script))
spec = importlib.util.spec_from_loader("akm", loader)
akm = importlib.util.module_from_spec(spec)
loader.exec_module(akm)

frames = {}
for line in Path(sys.argv[1]).read_text().splitlines():
    line = line.split("#")[0].strip().replace(" ", "")
    if line:
        b = bytes.fromhex(line)
        frames[b[0]] = b


def fake(devpath, rid, length=16):
    if rid not in frames:
        return None
    buf = bytearray(length)
    data = frames[rid][:length]
    buf[:len(data)] = data
    return bytes(buf)


akm.hid_get_feature = fake
print(json.dumps(akm.read_all_reports("/dev/null"), sort_keys=True))
