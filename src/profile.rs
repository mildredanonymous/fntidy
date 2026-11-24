use std::ffi::OsStr;

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use unicode_segmentation::UnicodeSegmentation;

use crate::windows_names;

#[derive(Clone, Copy, Default)]
pub enum Profile {
    #[default]
    Windows,
    Tidy,
    Posix,
}

impl Profile {
    pub fn parse(value: &OsStr) -> Option<Self> {
        match value.to_str()? {
            windows_names::PROFILE => Some(Self::Windows),
            "tidy" => Some(Self::Tidy),
            "posix" => Some(Self::Posix),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => windows_names::PROFILE,
            Self::Tidy => "tidy",
            Self::Posix => "posix",
        }
    }

    pub fn version(self) -> u32 {
        match self {
            Self::Windows | Self::Tidy | Self::Posix => 1,
        }
    }

    pub fn violations(self, name: &str) -> impl Iterator<Item = (&'static str, &'static str)> {
        let additional_rules = match self {
            Self::Windows => [None, None],
            Self::Posix => [
                (matches!(name, "" | "." | "..") || !name.chars().all(is_posix_character))
                    .then_some((
                        "POSIX_INVALID_NAME",
                        "Outside the portable POSIX filename rules.",
                    )),
                name.starts_with('-').then_some((
                    "POSIX_LEADING_HYPHEN",
                    "Starts with a hyphen that command-line tools may read as an option.",
                )),
            ],
            Self::Tidy => [
                name.graphemes(true)
                    .any(is_emoji)
                    .then_some(("TIDY_EMOJI", "Contains an emoji.")),
                (normalize_whitespace(name) != name).then_some((
                    "TIDY_WHITESPACE",
                    "Contains whitespace that can be simplified.",
                )),
            ],
        };
        windows_names::violations(name)
            .filter(move |_| !matches!(self, Self::Posix))
            .map(windows_names::Rule::diagnostic)
            .chain(additional_rules.into_iter().flatten())
    }

    pub fn repair(self, name: &str) -> Option<String> {
        match self {
            Self::Windows => windows_names::repair(name),
            Self::Posix => repair_posix(name),
            Self::Tidy => {
                let mut without_emoji = String::with_capacity(name.len());
                for grapheme in name.graphemes(true) {
                    without_emoji.push_str(if is_emoji(grapheme) { " " } else { grapheme });
                }
                let mut target = normalize_whitespace(&without_emoji);
                if target.is_empty() || (target.starts_with('.') && !name.starts_with('.')) {
                    target.insert(0, '_');
                }
                let target = windows_names::repair(&target).unwrap_or(target);
                (target != name).then_some(target)
            }
        }
    }
}

fn is_posix_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
}

fn repair_posix(name: &str) -> Option<String> {
    let mut target = String::with_capacity(name.len());
    let mut in_invalid_run = false;
    for character in name.nfc() {
        let replacement = match character {
            'ä' => "ae",
            'ö' => "oe",
            'ü' => "ue",
            'Ä' => "Ae",
            'Ö' => "Oe",
            'Ü' => "Ue",
            'ß' => "ss",
            'ẞ' => "SS",
            _ => "",
        };
        if !replacement.is_empty() {
            target.push_str(replacement);
            in_invalid_run = false;
        } else if is_posix_character(character) {
            target.push(character);
            in_invalid_run = false;
        } else {
            let mut decomposed = std::iter::once(character).nfd();
            if let Some(base) = decomposed.next().filter(char::is_ascii_alphabetic)
                && decomposed.all(is_combining_mark)
            {
                target.push(base);
                in_invalid_run = false;
            } else if !in_invalid_run {
                target.push('_');
                in_invalid_run = true;
            }
        }
    }
    if target.starts_with('-') {
        target.insert(0, '_');
    }
    (target != name
        && target
            .chars()
            .any(|character| character.is_ascii_alphanumeric()))
    .then_some(target)
}

fn is_emoji(grapheme: &str) -> bool {
    // Keep text-style symbols such as © unless they have an emoji presentation selector.
    emojis::get(grapheme)
        .is_some_and(|emoji| grapheme.chars().nth(1).is_some() || emoji.as_str() == grapheme)
}

fn normalize_whitespace(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut pending_space = false;
    for character in name.chars() {
        if character.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !result.is_empty() && !result.ends_with('.') && character != '.' {
            result.push(' ');
        }
        result.push(character);
        pending_space = false;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::Profile;

    #[test]
    fn posix_repairs_only_portable_filename_violations() {
        for (source, target) in [
            ("Überblick 2026.pdf", "Ueberblick_2026.pdf"),
            ("U\u{308}berblick 2026.pdf", "Ueberblick_2026.pdf"),
            ("Café ☕.txt", "Cafe_.txt"),
            ("Cafe\u{301} ☕.txt", "Cafe_.txt"),
            ("-entwurf.txt", "_-entwurf.txt"),
            ("Straße Æø.tar.gz", "Strasse_.tar.gz"),
            ("CÓN.txt", "CON.txt"),
        ] {
            assert_eq!(Profile::Posix.repair(source).as_deref(), Some(target));
            assert!(Profile::Posix.violations(source).next().is_some());
            assert!(Profile::Posix.violations(target).next().is_none());
            assert_eq!(Profile::Posix.repair(target), None);
        }
        for source in ["📁", "", ".", ".."] {
            assert!(Profile::Posix.violations(source).next().is_some());
            assert_eq!(Profile::Posix.repair(source), None);
        }
        for name in [".gitignore", "Report-2026.TXT", "_"] {
            assert!(Profile::Posix.violations(name).next().is_none());
            assert_eq!(Profile::Posix.repair(name), None);
        }
    }

    #[test]
    fn tidy_removes_complete_emoji_sequences_and_normalizes_whitespace() {
        for (source, target) in [
            ("Familie 👨‍👩‍👧‍👦.txt", "Familie.txt"),
            ("  März\u{a0}🇨🇭  2026.tar.gz", "März 2026.tar.gz"),
            ("Herz❤️Hand👋🏽.txt", "Herz Hand.txt"),
            ("Taste*️⃣.txt", "Taste.txt"),
            ("😀.txt", "_.txt"),
            ("🧪", "_"),
            ("CON 😀.txt", "_CON.txt"),
            ("Foto?.txt.", "Foto_.txt"),
        ] {
            assert_eq!(
                Profile::Tidy.repair(source).as_deref(),
                Some(target),
                "{source:?}"
            );
            assert_eq!(Profile::Tidy.repair(target), None);
            assert_eq!(Profile::Tidy.violations(target).next(), None);
        }
    }

    #[test]
    fn tidy_preserves_non_emoji_unicode_and_valid_punctuation() {
        for name in [
            ".gitignore",
            "© 2026.txt",
            "Grüezi [final] #1.txt",
            "日本語.txt",
        ] {
            assert_eq!(Profile::Tidy.repair(name), None, "{name:?}");
            assert_eq!(Profile::Tidy.violations(name).next(), None);
        }
    }
}
