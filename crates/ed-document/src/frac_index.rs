//! Fractional index strings for sibling order (spec §3.1).
//!
//! An index is a non-empty string of digits from a base-62 alphabet.
//! Ordering is plain lexicographic byte order with one invariant:
//! indexes never end with the minimum digit `0`, so a new index can
//! always be generated between any two existing ones by appending.

const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const MID: u8 = b'V'; // roughly the middle of the alphabet

fn digit_index(d: u8) -> usize {
    ALPHABET.iter().position(|&c| c == d).expect("invalid frac index digit")
}

/// Generate an index strictly between `lo` and `hi`.
/// `None` bounds mean the open ends of the sequence.
pub fn between(lo: Option<&str>, hi: Option<&str>) -> String {
    match (lo, hi) {
        (None, None) => (MID as char).to_string(),
        (Some(lo), None) => after(lo),
        (None, Some(hi)) => before(hi),
        (Some(lo), Some(hi)) => {
            debug_assert!(lo < hi, "between({lo:?}, {hi:?}) requires lo < hi");
            midpoint(lo.as_bytes(), hi.as_bytes())
        }
    }
}

fn after(lo: &str) -> String {
    // Increment the last digit if possible, else append the mid digit.
    let bytes = lo.as_bytes();
    let last = *bytes.last().unwrap();
    let idx = digit_index(last);
    if idx + 1 < ALPHABET.len() {
        let mut s = lo[..lo.len() - 1].to_string();
        s.push(ALPHABET[idx + 1] as char);
        s
    } else {
        let mut s = lo.to_string();
        s.push(MID as char);
        s
    }
}

fn before(hi: &str) -> String {
    // Find the first digit we can decrement; result must not end in '0'.
    let bytes = hi.as_bytes();
    for i in 0..bytes.len() {
        let idx = digit_index(bytes[i]);
        if idx > 1 {
            let mut s = hi[..i].to_string();
            s.push(ALPHABET[idx / 2.max(1)] as char);
            if s.as_bytes()[s.len() - 1] == b'0' {
                s.pop();
                s.push(ALPHABET[1] as char);
            }
            return s;
        }
        if idx == 1 {
            // digit is '1': go under it with "0" + mid
            let mut s = hi[..i].to_string();
            s.push('0');
            s.push(MID as char);
            return s;
        }
    }
    // hi is all zeros (shouldn't happen given the invariant) — go below with mid suffix
    let mut s = hi.to_string();
    s.pop();
    s.push('0');
    s.push(MID as char);
    s
}

fn midpoint(lo: &[u8], hi: &[u8]) -> String {
    // Walk the common prefix; at the first difference pick a digit in between
    // or recurse by extending lo.
    let mut prefix = Vec::new();
    let mut i = 0;
    loop {
        let l = lo.get(i).map(|&d| digit_index(d)).unwrap_or(0);
        let h = hi.get(i).map(|&d| digit_index(d)).unwrap_or(ALPHABET.len());
        if h - l > 1 {
            prefix.push(ALPHABET[(l + h) / 2]);
            return String::from_utf8(prefix).unwrap();
        }
        // digits are equal or adjacent: keep lo's digit and continue
        prefix.push(ALPHABET[l]);
        if h - l == 1 {
            // everything after this point only needs to be > rest-of-lo
            let rest = &lo[(i + 1).min(lo.len())..];
            let tail = append_above(rest);
            prefix.extend_from_slice(tail.as_bytes());
            return String::from_utf8(prefix).unwrap();
        }
        i += 1;
    }
}

/// Produce a string strictly greater than `rest` (interpreted as the
/// fractional part after a fixed prefix) but less than the next prefix value.
fn append_above(rest: &[u8]) -> String {
    let mut out = Vec::new();
    for (i, &d) in rest.iter().enumerate() {
        let idx = digit_index(d);
        if idx + 1 < ALPHABET.len() {
            // pick midpoint between this digit and the max
            out.push(ALPHABET[(idx + ALPHABET.len()) / 2]);
            let _ = i;
            return String::from_utf8(out).unwrap();
        }
        out.push(d);
    }
    out.push(MID);
    String::from_utf8(out).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(lo: Option<&str>, hi: Option<&str>) -> String {
        let m = between(lo, hi);
        if let Some(lo) = lo {
            assert!(lo < m.as_str(), "expected {lo:?} < {m:?}");
        }
        if let Some(hi) = hi {
            assert!(m.as_str() < hi, "expected {m:?} < {hi:?}");
        }
        assert!(!m.ends_with('0'), "index must not end with 0: {m:?}");
        m
    }

    #[test]
    fn basic_generation() {
        let first = check(None, None);
        let after = check(Some(&first), None);
        let before = check(None, Some(&first));
        check(Some(&before), Some(&first));
        check(Some(&first), Some(&after));
    }

    #[test]
    fn dense_insertion_left() {
        // repeatedly insert before the smallest — indexes stay valid
        let mut hi = between(None, None);
        for _ in 0..100 {
            hi = check(None, Some(&hi));
        }
    }

    #[test]
    fn dense_insertion_between() {
        let lo = between(None, None);
        let mut hi = between(Some(&lo), None);
        for _ in 0..100 {
            hi = check(Some(&lo), Some(&hi));
        }
        let mut lo2 = lo;
        let hi2 = hi.clone();
        for _ in 0..100 {
            lo2 = check(Some(&lo2), Some(&hi2));
        }
    }

    #[test]
    fn append_many() {
        let mut lo = between(None, None);
        for _ in 0..200 {
            lo = check(Some(&lo), None);
        }
    }
}
