#!/usr/bin/env python3
"""History fixtures for the end-to-end tests.

    gen_history.py seed  OUT   the anonymised real history (fixtures/history-seed.jsonl,
                               relative timestamps) rebased on now
    gen_history.py big   OUT   50 000 points of the same shape (5 min apart, real
                               field mix: voltage/voltage_valid/schema 2), ending now
    gen_history.py bad   OUT   corrupted file: invalid UTF-8, NUL bytes, truncated
                               lines, overflowing numbers, a 1 MiB line (#222, #223)
"""

import json
import random
import sys
import time
from pathlib import Path

SEED = Path(__file__).resolve().parent / "fixtures/history-seed.jsonl"


def seed_rows():
    return [json.loads(l) for l in SEED.read_text().splitlines() if l.strip()]


def write(out, rows):
    with open(out, "w") as f:
        for r in rows:
            f.write(json.dumps(r, separators=(",", ":")) + "\n")


def main():
    kind, out = sys.argv[1], sys.argv[2]
    now = int(time.time())
    if kind == "seed":
        write(out, [{**r, "ts": now + r["ts"]} for r in seed_rows()])
    elif kind == "big":
        rnd = random.Random(20261001)
        shapes = seed_rows()
        n, pct, rows = 50_000, 100.0, []
        for i in range(n):
            ts = now - (n - i) * 300
            pct -= rnd.uniform(0.0, 0.006)
            if pct < 4.0 or rnd.random() < 0.00005:  # battery swap
                pct = rnd.uniform(96, 100)
            shape = shapes[i % len(shapes)]
            r = {"ts": ts, "pct": round(pct, 1)}
            if "voltage" in shape:
                r["voltage"] = round(2.4 + pct / 100 * 0.8 + rnd.uniform(-0.01, 0.01), 3)
            if "voltage_valid" in shape:
                r["voltage_valid"] = shape["voltage_valid"]
            if "schema" in shape:
                r["schema"] = shape["schema"]
            rows.append(r)
        write(out, rows)
    elif kind == "bad":
        good = [{**r, "ts": now + r["ts"]} for r in seed_rows()]
        with open(out, "wb") as f:
            for i, r in enumerate(good):
                f.write((json.dumps(r) + "\n").encode())
                if i == 5:
                    f.write(b'{"ts":17908\xff\xfe55,"pct":90.0}\n')  # invalid UTF-8 (#222)
                if i == 10:
                    f.write(b"\0" * 64 + b"\n")
                if i == 15:
                    f.write(b'{"ts":1790858093,"pct":9')  # truncated, no newline
                    f.write(b"\n")
                if i == 20:
                    f.write(b'{"ts":18446744073709551615,"pct":1e308,"voltage":-1e308}\n')  # overflow (#223)
                if i == 25:
                    f.write(b'{"ts":0,"pct":-50}\n{"ts":1,"pct":"NaN"}\n[]\n"x"\n')
                if i == 30:
                    f.write(b'{"ts":1790858000,"pct":50,"pad":"' + b"A" * (1 << 20) + b'"}\n')
            f.write(b'{"ts":' + str(now).encode() + b',"pct":')  # torn last write
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
