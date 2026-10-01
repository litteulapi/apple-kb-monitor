//! AUDIT FINAL (2026-10-01) — fuzz déterministe (xorshift, sans dépendance)
//! des parseurs nouveaux lus par les programmes root : aucun ne doit paniquer,
//! et les invariants de la liste blanche doivent tenir sur toute entrée.

use akm_helper::breaker_state::BreakerState;
use akm_helper::hidctl::{self, keyboard_from_uevent, Mac};
use akm_helper::keymap::{parse_hwdb, render_hwdb, HWDB_HEADER};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[self.below(v.len())]
    }
}

const ATOMS: &[&str] = &[
    "\n", "\n", "\n", " ", "#", "=", ":", "*", "\\", "\"", "\0", "\r", "\t", "é", "0", "1", "3", "9", "a", "f", "F",
    "A", "schema", "mac", "open", "counter", "written_unix", "pid", "enabled", "true", "false", "-", "04:DB:56:CA:42:EE",
    "evdev:input:b0005v05ACp0256*", "evdev:input:b0003v05ACp0256*", "evdev:*", " KEYBOARD_KEY_7003f=f6", " KEYBOARD_KEY_c00b8=delete",
    " KEYBOARD_KEY_70029=power", " KEYBOARD_KEY_1=a", "RUN+=\"/bin/sh\"", "# profile: x", "HID_ID=0005:000005AC:00000256",
    "HID_UNIQ=04:db:56:ca:42:ee", "HID_NAME=x", "999999999999", "18446744073709551615", "4294967296", "-1",
];

fn gen(r: &mut Rng, header: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(h) = header {
        s.push_str(h);
        s.push('\n');
    }
    for _ in 0..r.below(40) {
        s.push_str(r.pick(ATOMS));
    }
    s
}

#[test]
fn breaker_state_parse_never_panics_and_roundtrips() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut ok = 0;
    const VALS: &[&str] = &["0", "1", "2", "3", "-", "04:DB:56:CA:42:EE", "04:db:56:ca:42:ee", "999999999999", "4294967296", "x", "", " 1", "1 "];
    for i in 0..200_000 {
        // Une entrée sur deux part d'un état valide dont les champs sont mutés.
        let s = if i % 2 == 0 {
            gen(&mut r, None)
        } else {
            let mut lines = vec![
                format!("schema={}", r.pick(VALS)),
                format!("mac={}", r.pick(VALS)),
                format!("open={}", r.pick(VALS)),
                format!("counter={}", r.pick(VALS)),
                format!("written_unix={}", r.pick(VALS)),
                format!("pid={}", r.pick(VALS)),
            ];
            if r.below(4) == 0 {
                lines.push(gen(&mut r, None));
            }
            if r.below(8) == 0 {
                let k = r.below(lines.len());
                lines.remove(k);
            }
            lines.join("\n") + "\n"
        };
        if let Ok(st) = BreakerState::parse(&s) {
            ok += 1;
            let again = BreakerState::parse(&st.render()).unwrap();
            assert_eq!(again, st);
            if let Some(m) = &st.mac {
                assert!(Mac::parse(m).is_ok(), "accepted mac {m:?} is not strict");
            }
        }
    }
    assert!(ok > 0, "generator never produced a valid state (test too weak)");
}

#[test]
fn hwdb_parse_never_panics_and_only_whitelisted_lines_survive() {
    let mut r = Rng(0xdead_beef_cafe_f00d);
    let mut ok = 0;
    for _ in 0..200_000 {
        let s = gen(&mut r, Some(HWDB_HEADER));
        if let Ok((profile, recs)) = parse_hwdb(&s) {
            ok += 1;
            let out = render_hwdb(profile.as_deref().unwrap_or("x"), &recs);
            for l in out.lines() {
                assert!(
                    l.is_empty()
                        || l.starts_with('#')
                        || l.starts_with("evdev:input:b0005v05ACp")
                        || l.starts_with(" KEYBOARD_KEY_"),
                    "line escaped the whitelist: {l:?}"
                );
            }
            // Ce qui est rendu se relit à l'identique (canonique).
            let (_, again) = parse_hwdb(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
            assert_eq!(again, recs);
        }
    }
    assert!(ok > 0);
}

#[test]
fn hid_suspend_conf_and_uevent_parsers_never_panic() {
    let mut r = Rng(0x0123_4567_89ab_cdef);
    for _ in 0..200_000 {
        let s = gen(&mut r, None);
        let _ = hidctl::parse_config(&s);
        let _ = hidctl::parse_main_pid(&s);
        if let Some(k) = keyboard_from_uevent(&s) {
            assert_eq!(k.mac.to_string().len(), 17);
        }
        let _ = Mac::parse(&s);
    }
}

/// Un nom de touche hors `KEYCODES` est refusé, mais tout nom de la liste du
/// noyau passe : `power`, `sleep`, `wakeup`, `sysrq`, `macro` compris. Le
/// hwdb installé (0644, système) s'applique à tous les comptes.
#[test]
fn hwdb_whitelist_accepts_power_and_sysrq_as_targets() {
    for name in ["power", "sleep", "wakeup", "sysrq", "macro", "power2"] {
        let src = format!("{HWDB_HEADER}\nevdev:input:b0005v05ACp0256*\n KEYBOARD_KEY_c00b8={name}\n");
        assert!(parse_hwdb(&src).is_ok(), "{name} refused");
    }
}
