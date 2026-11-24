use std::cmp::Ordering;

pub const PROFILE: &str = "windows";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    InvalidCharacter,
    TrailingDotOrSpace,
    ReservedDevice,
    NameCollision,
}

impl Rule {
    pub fn diagnostic(self) -> (&'static str, &'static str) {
        match self {
            Self::InvalidCharacter => (
                "WIN_INVALID_CHARACTER",
                "Contains a character Windows disallows in a name.",
            ),
            Self::TrailingDotOrSpace => (
                "WIN_TRAILING_DOT_OR_SPACE",
                "Ends in a period or ASCII space.",
            ),
            Self::ReservedDevice => (
                "WIN_RESERVED_DEVICE",
                "Uses a reserved Windows device name.",
            ),
            Self::NameCollision => (
                "WIN_NAME_COLLISION",
                "Matches another entry under Windows name comparison.",
            ),
        }
    }
}

fn is_invalid_character(character: char) -> bool {
    character <= '\u{1f}' || r#"<>:"/\|?*"#.contains(character)
}

pub fn repair(name: &str) -> Option<String> {
    if violations(name).next().is_none() {
        return None;
    }
    let mut target = comparison_name(name)
        .chars()
        .map(|character| {
            if is_invalid_character(character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    if target.is_empty() || violations(&target).any(|rule| rule == Rule::ReservedDevice) {
        target.insert(0, '_');
    }
    Some(target)
}

pub fn violations(name: &str) -> impl Iterator<Item = Rule> {
    let invalid_character = name.chars().any(is_invalid_character);
    let stem = comparison_name(name)
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    let upper_stem = stem.to_ascii_uppercase();
    let reserved_device = matches!(upper_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper_stem
            .strip_prefix("COM")
            .or_else(|| upper_stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            });

    [
        invalid_character.then_some(Rule::InvalidCharacter),
        name.ends_with(['.', ' '])
            .then_some(Rule::TrailingDotOrSpace),
        reserved_device.then_some(Rule::ReservedDevice),
    ]
    .into_iter()
    .flatten()
}

pub fn comparison_name(name: &str) -> &str {
    name.trim_end_matches(['.', ' '])
}

#[cfg(windows)]
pub fn compare(left: &[u16], right: &[u16]) -> Ordering {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CompareStringOrdinal(
            left: *const u16,
            left_len: i32,
            right: *const u16,
            right_len: i32,
            ignore_case: i32,
        ) -> i32;
    }

    // Handle empty names after trimming. Directory entry names fit in i32 lengths.
    if left.is_empty() || right.is_empty() {
        return left.len().cmp(&right.len());
    }
    // SAFETY: Both pointers refer to initialized UTF-16 slices of the supplied lengths.
    let result = unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    };
    match result {
        1 => Ordering::Less,
        2 => Ordering::Equal,
        3 => Ordering::Greater,
        _ => unreachable!("CompareStringOrdinal rejected valid directory entry names"),
    }
}

#[cfg(not(windows))]
pub fn compare(left: &[u16], right: &[u16]) -> Ordering {
    left.cmp(right)
}

pub fn comparison_units(name: &str) -> Vec<u16> {
    let name = comparison_name(name);
    #[cfg(not(windows))]
    let portable_name = name
        .chars()
        .map(|character| {
            let mut uppercase = character.to_uppercase();
            let first = uppercase.next().unwrap_or(character);
            if uppercase.next().is_none() {
                first
            } else {
                character
            }
        })
        .collect::<String>();
    #[cfg(not(windows))]
    let name = portable_name.as_str();
    name.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_names_without_changing_valid_unicode() {
        for (source, expected) in [
            ("CON.txt", "_CON.txt"),
            ("März 👩🏽‍💻?.pdf ", "März 👩🏽‍💻_.pdf"),
            ("Berichte.", "Berichte"),
            ("...", "_"),
        ] {
            assert_eq!(repair(source).as_deref(), Some(expected));
            assert_eq!(repair(expected), None);
        }
        assert_eq!(repair("März 👩🏽‍💻.pdf"), None);
    }

    #[test]
    fn detects_reserved_devices_with_extensions() {
        for name in ["CON.txt", "com¹.log", "LPT9.report", "AUX .md", "NUL"] {
            assert!(
                violations(name).any(|rule| rule == Rule::ReservedDevice),
                "{name}"
            );
        }
    }

    #[test]
    fn detects_invalid_characters_and_trailing_suffixes() {
        for name in ["a:b", "a\\b", "a\u{1f}b"] {
            assert!(
                violations(name).any(|rule| rule == Rule::InvalidCharacter),
                "{name}"
            );
        }
        for name in ["report.", "folder "] {
            assert!(
                violations(name).any(|rule| rule == Rule::TrailingDotOrSpace),
                "{name}"
            );
        }
    }

    #[test]
    fn preserves_valid_international_names() {
        for name in ["März 👩🏽‍💻.pdf", ".env", "COM10.txt", "LPT0", "漢字"] {
            assert_eq!(violations(name).next(), None, "{name}");
        }
    }

    #[test]
    fn compares_case_and_win32_suffixes() {
        assert_eq!(
            compare(
                &comparison_units("Report.txt."),
                &comparison_units("report.txt")
            ),
            Ordering::Equal
        );
        assert_ne!(
            compare(
                &comparison_units("é.txt"),
                &comparison_units("e\u{301}.txt")
            ),
            Ordering::Equal
        );
    }
}
