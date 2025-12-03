#[macro_export]
macro_rules! measure_time {
    ($b:expr) => {
        if cfg!(feature = "debug-print") {
            let start = std::time::Instant::now();
            let result = $b;
            let elapsed = start.elapsed();
            std::print!("[{}:{}] ", file!(), line!());
            std::println!("elapsed {:?}", elapsed);
            result
        } else {
            $b
        }
    };
}
