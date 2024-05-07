use crate::runner::execute_test_suite;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn run_e2e_test(test_path: &'static str) {
    let path = if cfg!(target_os = "linux") {
        format!("./{}", test_path)
    } else {
        format!("../{}", test_path)
    };
    let elapsed = Arc::new(Mutex::new(Duration::new(0, 0)));
    execute_test_suite(Path::new(path.as_str()), &elapsed, false, false, None).unwrap();
}

macro_rules! define_tests {
    (
        $( fn $test_name:ident($test_path:literal); )*
    ) => {
        $(
            #[test]
            fn $test_name() {
                run_e2e_test($test_path)
            }
        )*
    };
}

mod e2e_tests {
    use super::*;

    define_tests! {
        fn sstore_combinations_initial01_2_paris("tests/GeneralStateTests/stTimeConsuming/sstore_combinations_initial01_2_Paris.json");
    }
}

// #[test]
// #[ignore]
// fn test_sstore_combinations_initial01_2_paris() {
//     #[cfg(target_os = "macos")]
//     let path = Path::new(
//         "../tests/GeneralStateTests/stTimeConsuming/sstore_combinations_initial01_2_Paris.json",
//     );
//     #[cfg(not(target_os = "macos"))]
//     let path = Path::new(
//         "tests/GeneralStateTests/stTimeConsuming/sstore_combinations_initial01_2_Paris.json",
//     );
//     let elapsed = Arc::new(Mutex::new(Duration::new(0, 0)));
//     execute_test_suite(path, &elapsed, false, false).unwrap();
// }
