pub mod gas;
pub mod host;
pub mod inner_models;
pub mod instruction_result;
pub mod instructions;
pub mod translator;

pub(crate) const USE_GAS: bool = !cfg!(feature = "no_gas_measuring");
