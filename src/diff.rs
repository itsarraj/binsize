//! Pure size-diffing logic over already-parsed [`crate::elf`] data —
//! matches sections/symbols by name across two binaries and reports
//! what was added, removed, grew, or shrank.

use std::collections::BTreeMap;

use crate::elf::{Section, Symbol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeDelta {
    pub name: String,
    pub old_size: Option<u64>,
    pub new_size: Option<u64>,
}

impl SizeDelta {
    pub fn delta(&self) -> i64 {
        self.new_size.unwrap_or(0) as i64 - self.old_size.unwrap_or(0) as i64
    }

    pub fn kind(&self) -> &'static str {
        match (self.old_size, self.new_size) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            _ => "changed",
        }
    }
}

fn diff_by_name<'a>(
    old: impl Iterator<Item = (&'a str, u64)>,
    new: impl Iterator<Item = (&'a str, u64)>,
) -> Vec<SizeDelta> {
    let mut map: BTreeMap<String, (Option<u64>, Option<u64>)> = BTreeMap::new();
    for (name, size) in old {
        map.entry(name.to_string()).or_insert((None, None)).0 = Some(size);
    }
    for (name, size) in new {
        map.entry(name.to_string()).or_insert((None, None)).1 = Some(size);
    }
    let mut out: Vec<SizeDelta> = map
        .into_iter()
        .map(|(name, (old_size, new_size))| SizeDelta {
            name,
            old_size,
            new_size,
        })
        .filter(|d| d.delta() != 0)
        .collect();
    out.sort_by_key(|d| std::cmp::Reverse(d.delta().unsigned_abs()));
    out
}

pub fn diff_sections(old: &[Section], new: &[Section]) -> Vec<SizeDelta> {
    diff_by_name(
        old.iter().map(|s| (s.name.as_str(), s.size)),
        new.iter().map(|s| (s.name.as_str(), s.size)),
    )
}

pub fn diff_symbols(old: &[Symbol], new: &[Symbol]) -> Vec<SizeDelta> {
    diff_by_name(
        old.iter().map(|s| (s.name.as_str(), s.size)),
        new.iter().map(|s| (s.name.as_str(), s.size)),
    )
}

pub fn format_delta(delta: i64) -> String {
    if delta >= 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    }
}

pub fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB"];
    let mut size = n as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{size:.2} {}", UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(name: &str, size: u64) -> Section {
        Section {
            name: name.to_string(),
            sh_type: 1,
            size,
            is_nobits: false,
        }
    }

    fn sym(name: &str, size: u64) -> Symbol {
        Symbol {
            name: name.to_string(),
            size,
            is_func: true,
        }
    }

    #[test]
    fn diff_sections_detects_grown_section() {
        let old = vec![sec(".text", 1000)];
        let new = vec![sec(".text", 1500)];
        let diffs = diff_sections(&old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].delta(), 500);
        assert_eq!(diffs[0].kind(), "changed");
    }

    #[test]
    fn diff_sections_detects_added_section() {
        let old = vec![sec(".text", 1000)];
        let new = vec![sec(".text", 1000), sec(".rodata", 200)];
        let diffs = diff_sections(&old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].name, ".rodata");
        assert_eq!(diffs[0].kind(), "added");
        assert_eq!(diffs[0].delta(), 200);
    }

    #[test]
    fn diff_sections_detects_removed_section() {
        let old = vec![sec(".text", 1000), sec(".debug_info", 5000)];
        let new = vec![sec(".text", 1000)];
        let diffs = diff_sections(&old, &new);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].name, ".debug_info");
        assert_eq!(diffs[0].kind(), "removed");
        assert_eq!(diffs[0].delta(), -5000);
    }

    #[test]
    fn diff_sections_omits_unchanged_sections() {
        let old = vec![sec(".text", 1000)];
        let new = vec![sec(".text", 1000)];
        assert!(diff_sections(&old, &new).is_empty());
    }

    #[test]
    fn diff_sections_sorts_by_absolute_delta_descending() {
        let old = vec![sec("a", 100), sec("b", 100)];
        let new = vec![sec("a", 110), sec("b", 500)];
        let diffs = diff_sections(&old, &new);
        assert_eq!(diffs[0].name, "b");
        assert_eq!(diffs[1].name, "a");
    }

    #[test]
    fn diff_symbols_matches_by_name() {
        let old = vec![sym("process_request", 200)];
        let new = vec![sym("process_request", 350)];
        let diffs = diff_symbols(&old, &new);
        assert_eq!(diffs[0].delta(), 150);
    }

    #[test]
    fn format_delta_adds_explicit_plus_sign_for_positive() {
        assert_eq!(format_delta(500), "+500");
        assert_eq!(format_delta(-500), "-500");
        assert_eq!(format_delta(0), "+0");
    }

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.00 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MiB");
    }

    #[test]
    fn diff_by_name_handles_a_symbol_present_in_neither_after_filtering() {
        // Same name, same size in both -> no delta, must not appear.
        let old = vec![sym("stable_fn", 64)];
        let new = vec![sym("stable_fn", 64)];
        assert!(diff_symbols(&old, &new).is_empty());
    }
}
