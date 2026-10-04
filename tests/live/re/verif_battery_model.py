#!/usr/bin/env python3
"""Voltage → % model (BCM2042 firmware) vs real state of charge per chemistry. Without hardware.

* fw_pct(mv): reconstruction of 0x47 = piecewise linear interpolation of the smoothed
  voltage 0x49 over the factory table 0x5A (100/75/50/25 %), truncation, capped at 100.
  Below the last threshold: linear decrease to 0 at 0x5A[3] - (0x5A[2]-0x5A[3]) [hypothesis].
* Chemistry curves: voltage PER CELL at rest / low drain (~1 mA, 21 °C) as a function of the
  depth of discharge. APPROXIMATE values (± 0.03 V, ± 10 DoD points), summary of the datasheets
  Energizer E91/L91/NH15, Wikipedia "AA battery", not a measurement of these particular batteries.

usage: verif_battery_model.py [--mv 2950 ...]
"""
import argparse

TABLE_0x5A = [2954, 2506, 2404, 2054]  # mV, measured on unit AA:BB:CC:DD:EE:F1
LEVELS = [100, 75, 50, 25]

# DoD % -> V/cell (low drain)
CHEM = {
    "alcaline (E91/MN1500)": [(0, 1.58), (5, 1.53), (10, 1.50), (20, 1.46), (30, 1.42), (40, 1.38),
                              (50, 1.33), (60, 1.28), (70, 1.23), (80, 1.17), (90, 1.10), (95, 1.03), (100, 0.90)],
    "NiMH LSD (Eneloop)":    [(0, 1.40), (5, 1.32), (10, 1.29), (20, 1.27), (40, 1.25), (60, 1.23),
                              (80, 1.20), (90, 1.17), (95, 1.12), (100, 1.00)],
    "lithium Li-FeS2 (L91)": [(0, 1.78), (5, 1.70), (10, 1.68), (30, 1.65), (50, 1.62), (70, 1.58),
                              (85, 1.52), (90, 1.47), (95, 1.35), (100, 0.90)],
}


def fw_pct(mv, t=TABLE_0x5A, floor=True):
    if mv >= t[0]:
        return 100.0
    for (v_hi, p_hi), (v_lo, p_lo) in zip(zip(t, LEVELS), zip(t[1:], LEVELS[1:])):
        if mv >= v_lo:
            p = p_lo + (mv - v_lo) / (v_hi - v_lo) * (p_hi - p_lo)
            return float(int(p)) if floor else p
    v0 = t[3] - (t[2] - t[3])
    p = max(0.0, 25 * (mv - v0) / (t[3] - v0))
    return float(int(p)) if floor else p


def dod_from_cell_v(curve, v):
    """depth of discharge (%) for a per-cell voltage, decreasing curve."""
    if v >= curve[0][1]:
        return 0.0
    for (d1, v1), (d2, v2) in zip(curve, curve[1:]):
        if v2 <= v <= v1:
            return d1 + (v1 - v) / (v1 - v2) * (d2 - d1)
    return 100.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mv", type=int, nargs="*", default=[2991, 2986, 2950, 2935, 2900, 2800, 2775,
                                                           2700, 2600, 2506, 2450, 2404, 2200, 2054])
    a = ap.parse_args()
    names = list(CHEM)
    print("| pair voltage (mV) | V/cell | % firmware (0x47 rebuilt) | " +
          " | ".join("real %s" % n for n in names) + " |")
    print("|---|---|---|" + "---|" * len(names))
    for mv in a.mv:
        cell = mv / 2000
        real = ["%d %%" % round(100 - dod_from_cell_v(CHEM[n], cell)) for n in names]
        print("| %d | %.3f | %d %% | %s |" % (mv, cell, fw_pct(mv), " | ".join(real)))
    print()
    print("Firmware thresholds translated into real state:")
    for mv, lv in zip(TABLE_0x5A, LEVELS):
        cell = mv / 2000
        print("  %d %% firmware = %d mV = %.3f V/cell -> " % (lv, mv, cell) +
              ", ".join("%s %d %%" % (n.split()[0], round(100 - dod_from_cell_v(CHEM[n], cell))) for n in names))


if __name__ == "__main__":
    main()
