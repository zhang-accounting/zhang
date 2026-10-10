//! Golden files: a committed expected output, compared on every run and rewritten on request.
//!
//! Run the test with `UPDATE_GOLDEN=1` to write what the code produces now into the golden file; the diff of the
//! file is then the change under review. Without it the test fails on any difference and says how to update.

use std::path::Path;

use serde_json::Value;

/// the environment variable that makes the golden helpers rewrite their files
pub const UPDATE_VAR: &str = "UPDATE_GOLDEN";

/// Whether `UPDATE_GOLDEN=1` (or any value but `0` and empty) is set.
pub fn update_requested() -> bool {
    std::env::var(UPDATE_VAR).map(|value| !value.is_empty() && value != "0").unwrap_or(false)
}

/// Compare `actual` with the text of the golden file at `path`, or rewrite the file when an update is requested.
/// A missing file is a failure that names the variable to set. Line endings are compared as written.
pub fn assert_text(path: &Path, actual: &str) {
    if update_requested() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|error| panic!("{}: {error}", parent.display()));
        }
        std::fs::write(path, actual).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}; run the test with {UPDATE_VAR}=1 to write it", path.display()));
    if expected != actual {
        let first = expected
            .lines()
            .zip(actual.lines())
            .position(|(expected, actual)| expected != actual)
            .map(|line| line + 1)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()) + 1);
        panic!(
            "{} differs from what the code produces, first at line {first}; run the test with {UPDATE_VAR}=1 to rewrite it, then review the diff\n--- golden\n{expected}\n--- actual\n{actual}",
            path.display()
        );
    }
}

/// Compare `actual` with the JSON golden file at `path` (pretty-printed, one trailing newline), or rewrite it when
/// an update is requested.
pub fn assert_json(path: &Path, actual: &Value) {
    let mut text = serde_json::to_string_pretty(actual).expect("JSON serialises");
    text.push('\n');
    assert_text(path, &text)
}

/// The JSON golden file at `path`. Panics, naming the file, when it is missing or not JSON.
pub fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
