//! One history file: `akm_core::history::default_path()`.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

use akm_core::history::{self, HistoryEntry};
use serde_json::Value;

fn from_python(v: &Value) -> Option<HistoryEntry> {
    let ts = crate::when::parse_iso_local(v.get("t")?.as_str()?)?;
    let num = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x > 0.0)
    };
    let pct = num("fine").or_else(|| num("bat"))?;
    let voltage = num("volt").filter(|v| history::VOLTAGE_RANGE.contains(v));
    history::valid_sample(pct, voltage).then(|| HistoryEntry {
        ts,
        pct,
        voltage,
        voltage_valid: voltage.map(|_| false),
        ..HistoryEntry::default()
    })
}

pub fn parse_any(text: &str) -> Vec<HistoryEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("ts").is_some() {
            out.extend(history::parse(line));
        } else if let Some(e) = from_python(&v) {
            out.push(e);
        }
    }
    out
}

pub fn merge(
    existing: Vec<HistoryEntry>,
    incoming: Vec<HistoryEntry>,
) -> (Vec<HistoryEntry>, usize) {
    let mut seen: HashSet<u64> = existing.iter().map(|e| e.ts).collect();
    let mut all = existing;
    let mut added = 0;
    for e in incoming {
        if seen.insert(e.ts) {
            all.push(e);
            added += 1;
        }
    }
    all.sort_by_key(|e| e.ts);
    (all, added)
}

pub fn import_file(dest: &Path, source: &Path) -> io::Result<(usize, usize)> {
    let text = std::fs::read_to_string(source)?;
    let incoming = parse_any(&text);
    let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
    let ignored = lines - incoming.len();
    // The daemon appends to dest meanwhile: same lock as its writers.
    let _lock = history::lock(dest)?;
    let existing = std::fs::read_to_string(dest)
        .map(|c| history::parse(&c))
        .unwrap_or_default();
    let (all, added) = merge(existing, incoming);
    if added == 0 {
        return Ok((0, ignored));
    }
    if dest.exists() {
        std::fs::copy(dest, dest.with_extension("jsonl.prev"))?;
    }
    let mut out = Vec::new();
    for e in &all {
        writeln!(
            out,
            "{}",
            serde_json::to_string(e).map_err(io::Error::other)?
        )?;
    }
    akm_core::fsutil::write_atomic(dest, &out, akm_core::fsutil::Mode::Keep(0o666))?;
    Ok((added, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: &str = concat!(
        r#"{"t":"2026-09-01T10:00:00.5","mac":"AA:BB:CC:DD:EE:F1","bat":80,"fine":78,"volt":2.9,"adc":240,"rssi":0}"#,
        "\n",
        r#"{"t":"2026-09-01T10:05:00","bat":79}"#,
        "\n",
        r#"{"t":"2026-09-01T10:06:00","event":"wake","flags":["x"],"raw":19}"#,
        "\n",
        "not json\n",
        r#"{"t":"nonsense","bat":50}"#,
        "\n",
    );

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("akmctl-migrate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn python_lines_become_legacy_entries() {
        let e = parse_any(PY);
        assert_eq!(e.len(), 2, "events and garbage are skipped: {e:?}");
        assert!((e[0].pct - 78.0).abs() < 1e-9, "fine wins over bat");
        assert_eq!(e[0].voltage, Some(2.9));
        assert!(
            !e[0].voltage_reliable(),
            "the old voltage is not a measurement"
        );
        assert_eq!(e[0].reliable_voltage(), None);
        assert!((e[1].pct - 79.0).abs() < 1e-9);
        assert_eq!(e[1].voltage_valid, None);
        assert_eq!(e[1].ts - e[0].ts, 300);
    }

    #[test]
    fn current_format_lines_pass_through() {
        let line = serde_json::to_string(&HistoryEntry::measured(
            1_790_000_000,
            90.0,
            Some(3000),
            None,
        ))
        .unwrap();
        let e = parse_any(&format!("{line}\n"));
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].mv_0x46, Some(3000));
        assert!(e[0].voltage_reliable());
    }

    #[test]
    fn merge_is_idempotent_and_sorted() {
        let a = vec![HistoryEntry::sample(200, 50.0, None)];
        let b = vec![
            HistoryEntry::sample(100, 60.0, None),
            HistoryEntry::sample(200, 1.0, None),
        ];
        let (m, added) = merge(a, b.clone());
        assert_eq!(
            (added, m.iter().map(|e| e.ts).collect::<Vec<_>>()),
            (1, vec![100, 200])
        );
        assert!((m[1].pct - 50.0).abs() < 1e-9, "the existing line wins");
        assert_eq!(merge(m, b).1, 0);
    }

    #[test]
    fn import_writes_once_and_keeps_the_source() {
        let d = tmpdir("import");
        let (src, dst) = (d.join("py.jsonl"), d.join("state/history.jsonl"));
        std::fs::write(&src, PY).unwrap();
        assert_eq!(import_file(&dst, &src).unwrap(), (2, 3));
        assert_eq!(
            history::parse(&std::fs::read_to_string(&dst).unwrap()).len(),
            2
        );
        assert_eq!(
            std::fs::read_to_string(&src).unwrap(),
            PY,
            "source untouched"
        );
        assert_eq!(
            import_file(&dst, &src).unwrap().0,
            0,
            "second run adds nothing"
        );
        assert!(import_file(&dst, &d.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn import_keeps_the_samples_the_daemon_appends_meanwhile() {
        use akm_core::history::{History, SystemClock};
        let d = tmpdir("race");
        let (dst, src) = (d.join("history.jsonl"), d.join("in.jsonl"));
        let line = |ts| serde_json::to_string(&HistoryEntry::sample(ts, 50.0, None)).unwrap();
        std::thread::scope(|s| {
            let dst = &dst;
            s.spawn(move || {
                let h = History::new(dst, SystemClock);
                for i in 0..300 {
                    assert!(h
                        .append_entry(&HistoryEntry::sample(2_000_000 + i, 60.0, None))
                        .unwrap());
                }
            });
            for i in 0..100 {
                std::fs::write(&src, format!("{}\n", line(1_000_000 + i))).unwrap();
                import_file(dst, &src).unwrap();
            }
        });
        let kept: HashSet<u64> = history::parse(&std::fs::read_to_string(&dst).unwrap())
            .iter()
            .map(|e| e.ts)
            .collect();
        let lost = (0..300)
            .filter(|i| !kept.contains(&(2_000_000 + i)))
            .count();
        assert_eq!(lost, 0, "appended samples lost by the import");
        assert!((0..100).all(|i| kept.contains(&(1_000_000 + i))));
        let _ = std::fs::remove_dir_all(&d);
    }
}
