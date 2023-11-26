#![no_std]

extern crate alloc;
#[cfg(test)]
#[macro_use]
extern crate std;
extern crate wat;

mod arithmetic;
mod bitwise;
pub(crate) mod consts;
#[cfg(test)]
pub(crate) mod test_helper;
mod tests;

#[cfg(test)]
#[ctor::ctor]
fn log_init() {
    let init_res =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .try_init();
    if let Err(_) = init_res {
        println!("failed to init logger");
    }
}
