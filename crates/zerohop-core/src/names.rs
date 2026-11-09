//! Receiver-side handling of file names that arrive from the network.
//!
//! Implements protocol section 11.1. The order of the steps is security
//! critical: invisible characters must be removed *before* validation, or a
//! component such as `..` followed by U+200E would pass the `..` check and
//! become a real `..` afterwards.

use unicode_normalization::UnicodeNormalization;

/// Longest allowed path component in UTF-8 bytes.
pub const MAX_COMPONENT_BYTES: usize = 255;
/// Most components allowed in one path.
pub const MAX_COMPONENTS: usize = 64;
/// Longest allowed whole path in UTF-8 bytes.
pub const MAX_PATH_BYTES: usize = 1024;
/// Private working directory inside the receive folder. Senders may not use this name.
pub const TMP_DIR_NAME: &str = ".zerohop-tmp";

/// The file system conventions a name has to be safe for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    /// Windows reserved names and characters.
    Windows,
    /// macOS, where `:` is not usable in names.
    MacOs,
    /// Anything else.
    Other,
}

impl TargetOs {
    /// The conventions of the system this code is running on.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_vendor = "apple") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// Why a path from the network was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// A component was empty.
    #[error("empty path component")]
    Empty,
    /// A component was `.` or `..`.
    #[error("relative path component")]
    Relative,
    /// A component contained a separator or control character.
    #[error("forbidden character in path component")]
    ForbiddenCharacter,
    /// A component was too long.
    #[error("path component is too long")]
    ComponentTooLong,
    /// A component used the name reserved for Zerohop's working directory.
    #[error("reserved path component")]
    Reserved,
    /// The path had no components or too many.
    #[error("path has an invalid number of components")]
    BadComponentCount,
    /// The whole path was too long.
    #[error("path is too long")]
    PathTooLong,
}

/// Characters used to make a name display differently from what it is.
const fn is_disguise(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Code points some file systems skip when comparing names, so that a name
/// holding them can be the same name as one without them.
const fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{34F}'
            | '\u{61C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

fn validate(name: &str) -> Result<(), NameError> {
    // Joiners are kept in names, where some languages need them, but they do
    // not get to disguise the names that have a special meaning.
    let visible: String = name.chars().filter(|&c| !is_ignorable(c)).collect();
    if visible.is_empty() {
        return Err(NameError::Empty);
    }
    if visible == "." || visible == ".." {
        return Err(NameError::Relative);
    }
    if name.len() > MAX_COMPONENT_BYTES {
        return Err(NameError::ComponentTooLong);
    }
    if name
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err(NameError::ForbiddenCharacter);
    }
    if caseless::default_caseless_match_str(&visible, TMP_DIR_NAME) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

fn is_reserved_windows_stem(stem: &str) -> bool {
    let upper = stem.trim_end_matches(' ').to_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "CONIN$" | "CONOUT$" | "PRN" | "AUX" | "NUL"
    ) {
        return true;
    }
    match upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    {
        Some(rest) => {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'),
                _ => false,
            }
        }
        None => false,
    }
}

fn truncate_keeping_extension(name: String) -> String {
    if name.len() <= MAX_COMPONENT_BYTES {
        return name;
    }
    let ext = name
        .rfind('.')
        .filter(|&dot| dot > 0 && name.len() - dot <= 32)
        .and_then(|dot| name.get(dot..))
        .unwrap_or("");
    let keep = MAX_COMPONENT_BYTES.saturating_sub(ext.len());
    let mut stem = String::with_capacity(keep);
    for c in name.chars() {
        if stem.len() + c.len_utf8() > keep {
            break;
        }
        stem.push(c);
    }
    stem + ext
}

fn make_safe(name: &str, os: TargetOs) -> String {
    let forbidden = |c: char| match os {
        TargetOs::Windows => matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'),
        TargetOs::MacOs => c == ':',
        TargetOs::Other => false,
    };
    let mut chars: Vec<char> = name
        .chars()
        .map(|c| if forbidden(c) { '_' } else { c })
        .collect();
    for c in chars.iter_mut().rev() {
        if *c == '.' || *c == ' ' {
            *c = '_';
        } else {
            break;
        }
    }
    let mut out: String = chars.into_iter().collect();
    if os == TargetOs::Windows {
        let stem_end = out.find('.').unwrap_or(out.len());
        let reserved = out.get(..stem_end).is_some_and(is_reserved_windows_stem);
        if reserved {
            out.insert(stem_end, '_');
        }
    }
    truncate_keeping_extension(out)
}

/// Runs one path component through steps N2 to N6 of the protocol.
///
/// # Errors
/// Returns a [`NameError`] when the component must cause the offer to be declined.
pub fn sanitize_component(raw: &str, os: TargetOs) -> Result<String, NameError> {
    let stripped: String = raw.chars().filter(|&c| !is_disguise(c)).collect();
    let normalized: String = stripped.nfc().collect();
    validate(&normalized)?;
    let safe = make_safe(&normalized, os);
    validate(&safe)?;
    Ok(safe)
}

/// Sanitizes every component of a path and enforces the whole-path limits.
///
/// # Errors
/// Returns a [`NameError`] when the path must cause the offer to be declined.
pub fn sanitize_path(components: &[String], os: TargetOs) -> Result<Vec<String>, NameError> {
    if components.is_empty() || components.len() > MAX_COMPONENTS {
        return Err(NameError::BadComponentCount);
    }
    let cleaned = components
        .iter()
        .map(|c| sanitize_component(c, os))
        .collect::<Result<Vec<_>, _>>()?;
    let total = cleaned.iter().map(|c| c.len() + 1).sum::<usize>() - 1;
    if total > MAX_PATH_BYTES {
        return Err(NameError::PathTooLong);
    }
    Ok(cleaned)
}

/// Makes a device name safe to show: no direction tricks or control characters.
#[must_use]
pub fn display_name(raw: &str) -> String {
    raw.chars()
        .filter(|&c| !is_disguise(c) && !c.is_control())
        .nfc()
        .collect()
}

/// A key for comparing names the way a case-insensitive file system would.
#[must_use]
pub fn fold_key(name: &str) -> String {
    caseless::default_case_fold_str(&name.nfc().collect::<String>())
        .nfc()
        .collect()
}

const PROGRAM_EXTENSIONS: &[&str] = &[
    "exe", "msi", "msix", "appx", "bat", "cmd", "com", "scr", "ps1", "vbs", "vbe", "js", "jse",
    "wsf", "hta", "lnk", "reg", "dll", "cpl", "jar", "app", "command", "pkg", "mpkg", "dmg",
    "terminal", "scpt", "workflow", "apk", "apks", "xapk", "aab", "sh", "py", "pl", "rb",
];

/// Folders that are themselves applications, so what is inside them is too.
const BUNDLE_EXTENSIONS: &[&str] = &["app", "pkg", "mpkg", "workflow"];

fn has_extension_in(name: &str, list: &[&str]) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        let ext = fold_key(ext);
        list.contains(&ext.as_str())
    })
}

/// Whether any component of a sanitized path has a program or script extension.
#[must_use]
pub fn is_program_path(components: &[String]) -> bool {
    components
        .iter()
        .any(|name| has_extension_in(name, PROGRAM_EXTENSIONS))
}

/// Whether this file was presented to the owner as a program: its own name
/// says so, or it is inside an application bundle. A folder merely named like
/// a program says nothing about the files in it.
#[must_use]
pub fn declared_as_program(components: &[String]) -> bool {
    let Some((name, folders)) = components.split_last() else {
        return false;
    };
    has_extension_in(name, PROGRAM_EXTENSIONS)
        || folders
            .iter()
            .any(|folder| has_extension_in(folder, BUNDLE_EXTENSIONS))
}

/// Whether the first bytes of a file identify it as an executable or script.
#[must_use]
pub fn looks_executable(prefix: &[u8]) -> bool {
    const MACH_O: [[u8; 4]; 5] = [
        [0xFE, 0xED, 0xFA, 0xCE],
        [0xFE, 0xED, 0xFA, 0xCF],
        [0xCE, 0xFA, 0xED, 0xFE],
        [0xCF, 0xFA, 0xED, 0xFE],
        [0xCA, 0xFE, 0xBA, 0xBE],
    ];
    prefix.starts_with(b"MZ")
        || prefix.starts_with(b"\x7FELF")
        || prefix.starts_with(b"#!")
        || MACH_O.iter().any(|m| prefix.starts_with(m))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn win(s: &str) -> Result<String, NameError> {
        sanitize_component(s, TargetOs::Windows)
    }

    #[test]
    fn rejects_c1_control_characters() {
        for raw in ["a\u{85}b", "_\u{94}", "\u{9B}31m"] {
            assert_eq!(win(raw), Err(NameError::ForbiddenCharacter), "{raw:?}");
        }
    }

    #[test]
    fn rejects_special_names_disguised_with_ignorable_characters() {
        for raw in [".\u{200C}.", "\u{FEFF}..", "\u{AD}."] {
            assert_eq!(
                sanitize_component(raw, TargetOs::Other),
                Err(NameError::Relative),
                "{raw:?}"
            );
        }
        assert_eq!(
            sanitize_component(".zero\u{200D}hop-tmp", TargetOs::Other),
            Err(NameError::Reserved)
        );
        assert_eq!(
            sanitize_component("\u{200B}", TargetOs::Other),
            Err(NameError::Empty)
        );
    }

    #[test]
    fn keeps_joiners_in_ordinary_names() {
        let persian = "\u{645}\u{6CC}\u{200C}\u{62E}\u{648}\u{627}\u{647}\u{645}.txt";
        assert_eq!(
            sanitize_component(persian, TargetOs::Other).as_deref(),
            Ok(persian)
        );
    }

    #[test]
    fn rejects_dot_dot_hidden_behind_an_invisible_character() {
        for raw in ["..\u{200E}", "\u{202E}..", ".\u{200F}.", "\u{061C}."] {
            assert_eq!(
                sanitize_component(raw, TargetOs::Other),
                Err(NameError::Relative),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn rejects_a_name_that_is_only_invisible_characters() {
        assert_eq!(
            sanitize_component("\u{200E}\u{202E}", TargetOs::Other),
            Err(NameError::Empty)
        );
    }

    #[test]
    fn rejects_separators_and_control_characters() {
        for raw in ["a/b", "a\\b", "a\0b", "a\nb", "a\u{7F}"] {
            assert_eq!(
                sanitize_component(raw, TargetOs::Other),
                Err(NameError::ForbiddenCharacter),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn rejects_the_private_directory_name_in_any_case() {
        assert_eq!(
            sanitize_component(".ZeroHop-TMP", TargetOs::Other),
            Err(NameError::Reserved)
        );
    }

    #[test]
    fn removes_right_to_left_override_from_a_disguised_name() {
        assert_eq!(
            sanitize_component("photo\u{202E}gpj.exe", TargetOs::Other).unwrap(),
            "photogpj.exe"
        );
    }

    #[test]
    fn rewrites_windows_reserved_stems_on_the_stem() {
        assert_eq!(win("CON.txt").unwrap(), "CON_.txt");
        assert_eq!(win("nul").unwrap(), "nul_");
        assert_eq!(win("COM1.tar.gz").unwrap(), "COM1_.tar.gz");
        assert_eq!(win("LPT²").unwrap(), "LPT²_");
        assert_eq!(win("CONIN$").unwrap(), "CONIN$_");
    }

    #[test]
    fn treats_a_reserved_stem_with_trailing_spaces_as_reserved() {
        assert_eq!(win("NUL .txt").unwrap(), "NUL _.txt");
    }

    #[test]
    fn leaves_similar_names_alone_on_windows() {
        assert_eq!(win("CONSOLE.txt").unwrap(), "CONSOLE.txt");
        assert_eq!(win("COM10").unwrap(), "COM10");
    }

    #[test]
    fn replaces_characters_windows_forbids() {
        assert_eq!(win("a:b?c*.txt").unwrap(), "a_b_c_.txt");
    }

    #[test]
    fn replaces_only_the_colon_on_macos() {
        assert_eq!(
            sanitize_component("a:b?.txt", TargetOs::MacOs).unwrap(),
            "a_b?.txt"
        );
    }

    #[test]
    fn replaces_trailing_dots_and_spaces() {
        assert_eq!(
            sanitize_component("name. .", TargetOs::Other).unwrap(),
            "name___"
        );
    }

    #[test]
    fn normalizes_to_nfc() {
        assert_eq!(
            sanitize_component("e\u{301}.txt", TargetOs::Other).unwrap(),
            "\u{E9}.txt"
        );
    }

    #[test]
    fn rejects_an_over_long_component() {
        let long = "a".repeat(256);
        assert_eq!(
            sanitize_component(&long, TargetOs::Other),
            Err(NameError::ComponentTooLong)
        );
    }

    #[test]
    fn keeps_the_extension_when_a_suffix_forces_truncation() {
        let name = format!("CON{}.txt", "");
        assert_eq!(win(&name).unwrap(), "CON_.txt");
        let at_limit = format!("nul{}", ".".to_string() + &"x".repeat(251));
        let safe = win(&at_limit).unwrap();
        assert!(safe.len() <= MAX_COMPONENT_BYTES);
        assert!(safe.starts_with("nul_"));
    }

    #[test]
    fn rejects_too_many_components() {
        let path = vec!["a".to_string(); MAX_COMPONENTS + 1];
        assert_eq!(
            sanitize_path(&path, TargetOs::Other),
            Err(NameError::BadComponentCount)
        );
    }

    #[test]
    fn rejects_an_empty_path() {
        assert_eq!(
            sanitize_path(&[], TargetOs::Other),
            Err(NameError::BadComponentCount)
        );
    }

    #[test]
    fn rejects_a_path_over_the_total_limit() {
        let path = vec!["a".repeat(250); 5];
        assert_eq!(
            sanitize_path(&path, TargetOs::Other),
            Err(NameError::PathTooLong)
        );
    }

    #[test]
    fn detects_programs_in_any_case_and_any_component() {
        assert!(is_program_path(&["EVIL.BAT".to_string()]));
        assert!(is_program_path(&["run.Ps1".to_string()]));
        assert!(is_program_path(&[
            "Foo.app".to_string(),
            "Contents".to_string(),
            "x".to_string()
        ]));
        assert!(!is_program_path(&["photo.jpg".to_string()]));
    }

    #[test]
    fn detects_executable_magic_numbers() {
        assert!(looks_executable(b"MZ\x90\0"));
        assert!(looks_executable(b"\x7FELF\x02"));
        assert!(looks_executable(b"#!/bin/sh"));
        assert!(looks_executable(&[0xCF, 0xFA, 0xED, 0xFE, 7]));
        assert!(!looks_executable(b"\xFF\xD8\xFF\xE0"));
    }

    #[test]
    fn fold_key_treats_case_and_normalization_as_equal() {
        assert_eq!(fold_key("FILE"), fold_key("file"));
        assert_eq!(fold_key("e\u{301}"), fold_key("\u{E9}"));
    }

    fn any_os() -> impl Strategy<Value = TargetOs> {
        prop_oneof![
            Just(TargetOs::Windows),
            Just(TargetOs::MacOs),
            Just(TargetOs::Other)
        ]
    }

    fn hostile_name() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just('.'),
                Just('/'),
                Just('\\'),
                Just(':'),
                Just(' '),
                Just('\u{200E}'),
                Just('\u{202E}'),
                Just('\u{2066}'),
                Just('\u{0}'),
                Just('C'),
                Just('O'),
                Just('N'),
                Just('a'),
                any::<char>(),
            ],
            0..40,
        )
        .prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        #[test]
        fn output_is_always_a_single_safe_component(name in hostile_name(), os in any_os()) {
            if let Ok(safe) = sanitize_component(&name, os) {
                prop_assert!(safe != "." && safe != "..");
                prop_assert!(!safe.is_empty());
                prop_assert!(safe.len() <= MAX_COMPONENT_BYTES);
                prop_assert!(!safe.contains('/') && !safe.contains('\\'));
                prop_assert!(!safe.chars().any(|c| c < ' ' || c == '\x7F'));
                prop_assert!(!safe.chars().any(is_disguise));
                prop_assert!(!safe.ends_with('.') && !safe.ends_with(' '));
            }
        }

        #[test]
        fn windows_output_is_never_a_reserved_device_name(name in hostile_name()) {
            if let Ok(safe) = sanitize_component(&name, TargetOs::Windows) {
                let stem = safe.split('.').next().unwrap();
                prop_assert!(!is_reserved_windows_stem(stem), "{safe:?}");
                prop_assert!(!safe.contains(':'));
            }
        }
    }
}
