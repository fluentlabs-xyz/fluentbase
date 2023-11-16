extern crate alloc;
extern crate core;

pub(crate) const USE_GAS: bool = !cfg!(feature = "no_gas_measuring");

pub mod compiler;

#[cfg(test)]
mod compiler_tests;
pub mod macros;
pub mod primitives;
pub mod translator;
pub mod utilities;

#[ctor::ctor]
fn log_init() {
    let init_res =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();
    if let Err(e) = init_res {
        println!("failed to init logger: {}", e.to_string());
    }
}
