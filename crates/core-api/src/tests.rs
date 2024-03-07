#[macro_use]
mod macros {
    #[macro_export]
    macro_rules! X {
        () => {
            Y!();
        };
    }
    #[macro_export]
    macro_rules! Y {
        () => {};
    }
}

#[test]
fn test_test() {
    X!();
}
