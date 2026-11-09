//! Name handling: whatever comes out must be safe to give to the file system.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zerohop_core::names::*;

fn assert_safe(name: &str, os: TargetOs) {
    assert!(!name.is_empty());
    assert!(name != "." && name != "..", "{name:?}");
    assert!(name.len() <= 255, "{name:?}");
    assert!(
        !name.chars().any(|c| c == '/' || c == '\\' || c.is_control()),
        "forbidden character in {name:?}"
    );
    assert!(
        !name.chars().any(|c| matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')),
        "disguise character in {name:?}"
    );
    let visible: String = name.chars().filter(|c| !ignorable(*c)).collect();
    assert!(visible != "." && visible != ".." && !visible.is_empty(), "looks relative: {name:?}");
    assert_ne!(fold_key(&visible), TMP_DIR_NAME, "reserved name: {name:?}");
    match os {
        TargetOs::Windows => {
            assert!(!name.chars().any(|c| "<>:\"|?*".contains(c)), "{name:?}");
            assert!(!name.ends_with('.') && !name.ends_with(' '), "{name:?}");
            let stem = name.split('.').next().unwrap_or("").trim_end().to_uppercase();
            assert!(
                !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"),
                "reserved on Windows: {name:?}"
            );
        }
        TargetOs::MacOs => assert!(!name.contains(':'), "{name:?}"),
        TargetOs::Other => {}
    }
}

/// Code points some file systems skip when comparing names.
fn ignorable(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}')
}

fuzz_target!(|input: (Vec<String>, String)| {
    let (parts, single) = input;
    for os in [TargetOs::Windows, TargetOs::MacOs, TargetOs::Other] {
        if let Ok(clean) = sanitize_component(&single, os) {
            assert_safe(&clean, os);
            assert_eq!(sanitize_component(&clean, os).as_deref(), Ok(clean.as_str()), "not stable");
        }
        if let Ok(path) = sanitize_path(&parts, os) {
            assert_eq!(path.len(), parts.len());
            assert!(path.join("/").len() <= MAX_PATH_BYTES);
            for name in &path {
                assert_safe(name, os);
            }
            let _ = is_program_path(&path);
        }
    }
    let shown = display_name(&single);
    assert!(!shown.chars().any(char::is_control), "{shown:?}");
    assert!(!shown.chars().any(|c| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')), "{shown:?}");
    let _ = fold_key(&single);
    let _ = looks_executable(single.as_bytes());
});
