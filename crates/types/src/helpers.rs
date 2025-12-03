#[macro_export]
macro_rules! measure_time {
    ($b:expr) => {{
        let start = std::time::Instant::now();
        let result = $b;
        std::print!("[{}:{}] ", file!(), line!());
        std::println!("elapsed {:?}", start.elapsed());
        result
    }};
}
