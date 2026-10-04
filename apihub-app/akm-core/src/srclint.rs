//! Architecture checks on source text, for tests: what a file's production code may not call.

/// `src` up to its `#[cfg(test)] mod`, without comment lines and without whitespace, so a
/// check does not depend on formatting; a `#[cfg(test)]` item before it stays in.
#[doc(hidden)]
#[must_use]
pub fn prod_tokens(src: &str) -> String {
    let mut end = src.len();
    for (i, _) in src.match_indices("#[cfg(test)]") {
        if src[i + "#[cfg(test)]".len()..]
            .trim_start()
            .starts_with("mod ")
        {
            end = i;
            break;
        }
    }
    src[..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_production_tokens_only() {
        let src = "/// doc\nfn  a( x: u8 ) {\n    // note\n    b(x) }\n#[cfg(test)]\nmod t {}\n";
        assert_eq!(prod_tokens(src), "fna(x:u8){b(x)}");
        let src = "#[cfg(test)]\nfn t() {}\nfn p() {}\n#[cfg(test)]\nmod tests {}\n";
        assert_eq!(prod_tokens(src), "#[cfg(test)]fnt(){}fnp(){}");
    }
}
