//! The vectors are the spec's, and of this SDK's version (SDK.md §5.2).

mod common;

#[test]
fn the_vectors_are_this_sdks_version() {
    assert_eq!(common::vectors()["version"].as_str(), Some(hotty::VERSION));
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
