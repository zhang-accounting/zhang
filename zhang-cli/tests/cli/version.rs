//! `zhang --version` prints the release version (#491): `ZHANG_BUILD_VERSION`, which build.rs takes from the
//! `.build_version` file the release workflow writes, falling back to the crate version. The v0.2.0 binary printed
//! `zhang 0.1.0`, the crate version, while `zhang update` and the server log used 0.2.0.

use std::process::Command;

#[test]
fn version_prints_the_build_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_zhang")).arg("--version").output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    // build.rs sets `ZHANG_BUILD_VERSION` for this test as for the binary; in a release build it is the tag's
    // version, not the crate version
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("zhang {}", env!("ZHANG_BUILD_VERSION"))
    );
}
