use trybuild::TestCases;

#[test]
fn function_id_tests() {
    let t = TestCases::new();
    t.pass("tests/01-basic-usage.rs");
    t.pass("tests/02-hex-string.rs");
    t.pass("tests/03-byte-array.rs");
    t.pass("tests/04-with-validate.rs");
    t.compile_fail("tests/05-invalid-signature.rs");
    t.compile_fail("tests/06-invalid-hex.rs");
    t.compile_fail("tests/07-invalid-byte-array.rs");
}
