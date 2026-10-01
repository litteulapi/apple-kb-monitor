#![no_main]
//! Rapports Feature/Input arbitraires : jamais de panique.
use akm_core::decode::{build_report, DecodeOptions, HidSource};
use akm_core::read_policy::build_report_safe;
use akm_core::report::KbWake;
use libfuzzer_sys::fuzz_target;
use std::io;
use std::time::Instant;

const UEVENT: &str = "DRIVER=apple\nHID_ID=0005:000005AC:00000256\nHID_NAME=K\nHID_UNIQ=04:db:56:ca:42:ee\n";

/// data = suite de blocs [id, len, octets...] : la première occurrence d'un id gagne.
struct Src(Vec<(u8, Vec<u8>)>);
impl HidSource for Src {
    fn feature(&self, id: u8) -> io::Result<Vec<u8>> {
        self.0
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

fuzz_target!(|data: &[u8]| {
    let mut frames = Vec::new();
    let mut d = data;
    while d.len() >= 2 {
        let (id, n) = (d[0], usize::from(d[1]));
        let take = n.min(d.len() - 2);
        frames.push((id, d[2..2 + take].to_vec()));
        d = &d[2 + take..];
    }
    let s = Src(frames);
    let _ = DecodeOptions::default();
    let _ = build_report(UEVENT, None, &s, KbWake::default());
    let _ = build_report_safe(UEVENT, None, &s, KbWake::default(), Instant::now());
    let _ = akm_core::passive::decode(data);
    let _ = akm_core::discover::decode_wake(data);
    let _ = akm_core::calibration::parse_calibration(data);
});
