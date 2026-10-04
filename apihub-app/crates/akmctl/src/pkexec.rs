//! Exit codes pkexec itself returns (`man pkexec`, RETURN VALUE).

use akm_core::tr;

/// The failure pkexec reports for `code`, or `None` when the code is the program's own.
pub fn failure(code: Option<i32>, what: &str) -> Option<String> {
    match code {
        Some(126) => Some(tr!("authentication dismissed")),
        Some(127) => Some(tr!(
            "not authorized, authentication failed or {what} not found",
            what = what
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::failure;

    #[test]
    fn dismissal_is_126_and_refusal_is_127() {
        assert_eq!(
            failure(Some(126), "x").as_deref(),
            Some("authentication dismissed")
        );
        let e = failure(Some(127), "akm-helper").unwrap();
        assert!(
            e.starts_with("not authorized") && e.contains("akm-helper"),
            "{e}"
        );
        assert_eq!(failure(Some(0), "x"), None);
        assert_eq!(failure(Some(1), "x"), None);
        assert_eq!(failure(None, "x"), None);
    }
}
