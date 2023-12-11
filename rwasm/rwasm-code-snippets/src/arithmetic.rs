#[cfg(feature = "arithmetic_mod")]
pub mod a_mod;
#[cfg(feature = "arithmetic_add")]
pub mod add;
#[cfg(feature = "arithmetic_div")]
pub mod div;
#[cfg(feature = "arithmetic_mul")]
pub mod mul;
#[cfg(feature = "arithmetic_sdiv")]
pub mod sdiv;
#[cfg(feature = "arithmetic_signextend")]
mod signextend;
#[cfg(feature = "arithmetic_sub")]
pub mod sub;
#[cfg(feature = "arithmetic_sub_global")]
mod sub_global;
