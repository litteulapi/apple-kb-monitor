#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let (cfg, _) = akm_core::config::parse(s);
        assert!(cfg.alerts.hysteresis.is_finite());
        let _ = akm_core::alias::validate(s);
        let _ = akm_core::chemistry::Chemistry::parse(s);
    }
});
