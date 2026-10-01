#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    let e = akm_core::history::parse(&s);
    let _ = akm_core::batteries::battery_sets(&e);
    let _ = akm_core::batteries::replacements(&e);
    // estimations : tolérées (défauts connus : voir docs/AUDIT-OUTILLE.md), on
    // vérifie seulement l'absence de panique hors débordement documenté.
    let _ = akm_core::history::estimate_remaining(&e);
});
