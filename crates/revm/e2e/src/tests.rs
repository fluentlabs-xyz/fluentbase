use crate::runner::execute_test_suite;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
#[ignore]
fn test_sstore_combinations_initial01_2_paris() {
    let path = Path::new(
        "../tests/GeneralStateTests/stTimeConsuming/sstore_combinations_initial01_2_Paris.json",
    );
    let elapsed = Arc::new(Mutex::new(Duration::new(0, 0)));
    execute_test_suite(path, &elapsed, false, false).unwrap();
}
