//! The vectors are the spec's, and of this SDK's version (SDK.md §5.2), and
//! so are the crates.

mod common;

#[test]
fn the_vectors_are_this_sdks_version() {
    assert_eq!(common::vectors()["version"].as_str(), Some(hotty::VERSION));
}

/// The crates' MAJOR.MINOR is the spec version this SDK speaks, and PATCH
/// their own (gov P-5 rule 6): 0.6.x speaks hotty 0.6.
#[test]
fn the_crates_are_this_sdks_version() {
    let v = env!("CARGO_PKG_VERSION");
    let major_minor: Vec<&str> = v.split('.').take(2).collect();
    assert_eq!(major_minor.join("."), hotty::VERSION, "the crates are {v}");
}

/// The copy in testdata is the spec's, when a checkout of the hotty
/// repository is at hand: HOTTY_DIR, or one next to this repository.
#[test]
fn the_vectors_are_current() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = match std::env::var_os("HOTTY_DIR") {
        Some(d) => std::path::PathBuf::from(d),
        None => manifest.join("../../../hotty"),
    };
    let spec = dir.join("conformance/vectors.json");
    let Ok(theirs) = std::fs::read(&spec) else {
        eprintln!("no hotty checkout at {}: not compared", dir.display());
        return;
    };
    let ours = std::fs::read(manifest.join("tests/testdata/vectors.json")).unwrap();
    assert!(
        ours == theirs,
        "tests/testdata/vectors.json differs from {}: run `make vectors`",
        spec.display()
    );
}
