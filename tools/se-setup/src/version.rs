//! Version numbers: "v1.4.0", "1.4.0", "beta-0.2.0", "1.4" compare numerically.

/// The numeric parts of a tag ("v1.10.2" -> [1, 10, 2]); None when there are none.
pub fn parse(tag: &str) -> Option<Vec<u32>> {
    let start = tag.find(|c: char| c.is_ascii_digit())?;
    let t = &tag[start..];
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let v: Vec<u32> = t[..end].split('.').filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0)).collect();
    (!v.is_empty()).then_some(v)
}

/// "1.4.0" from "v1.4.0".
pub fn plain(tag: &str) -> String {
    parse(tag).map(|v| v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".")).unwrap_or_else(|| tag.to_string())
}

/// Is `latest` newer than `current`? An unknown current version counts as older.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse(latest), parse(current)) else { return parse(latest).is_some() };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically() {
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(!is_newer("v1.4.0", "1.4"));
        assert!(!is_newer("1.3.9", "1.4.0"));
        assert!(is_newer("v1.4.1", ""));
        assert_eq!(plain("v1.4.0"), "1.4.0");
        assert_eq!(parse("beta-0.2.0"), Some(vec![0, 2, 0]));
        assert_eq!(parse("1.4.0.0"), Some(vec![1, 4, 0, 0]));
    }
}
