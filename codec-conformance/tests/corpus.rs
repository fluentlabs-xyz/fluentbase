//! The corpus, run through the codec.
//!
//! The generated cases carry the corpus's own type shapes, which nest up to seven levels - that is
//! the thing under test, not something to factor out.
#![allow(clippy::type_complexity)]

include!(concat!(env!("OUT_DIR"), "/cases.rs"));

/// The corpus is sha256-pinned by `make vectors`, so these counts are fixed. They are asserted
/// because "no divergences" is also what an empty run prints: if `build.rs` ever emitted nothing -
/// a parser change, a corpus that failed to download, a filter that matched too little - both tests
/// below would pass green while checking nothing at all.
const TUPLE_PATH_VECTORS: usize = 1880;
const DERIVE_PATH_VECTORS: usize = 576;

#[test]
fn the_corpus_encodes_as_solc_does() {
    let mut report = Report::default();
    tuple_path_cases(&mut report);
    assert_eq!(
        report.checked, TUPLE_PATH_VECTORS,
        "the generated corpus changed size; if that is intended, update the constant"
    );
    report.assert_clean("corpus, tuple path");
}

/// The same vectors, with every Solidity tuple as a `#[derive(Codec)]` struct.
///
/// `sol_to_rust` maps `tuple(...)` to an anonymous Rust tuple, so the test above never reaches the
/// derive - and the largest defect found so far, an array striding by the wrong `HEADER_SIZE`,
/// lived there. The two paths are independent
/// implementations of the same rules and have been measured disagreeing, so both need covering.
#[test]
fn the_corpus_encodes_as_solc_does_through_the_derive() {
    let mut report = Report::default();
    derive_path_cases(&mut report);
    assert_eq!(
        report.checked, DERIVE_PATH_VECTORS,
        "the generated corpus changed size; if that is intended, update the constant"
    );
    report.assert_clean("corpus, derive path");
}
