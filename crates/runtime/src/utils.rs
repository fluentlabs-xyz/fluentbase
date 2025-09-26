#[cfg(feature = "inter-process-lock")]
mod inter_process_lock;
#[cfg(feature = "inter-process-lock")]
pub use inter_process_lock::inter_process_lock;
