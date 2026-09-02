use crate::{BlockListAllocator, NativeAPI, RwasmContext};
use alloc::vec::Vec;

/// Serializes this Wasm instance's LLVM counters into the normal host output buffer.
///
/// This function is public only so [`crate::guest_coverage_entrypoint!`] can call it from a
/// contract crate. The feature is CI-only; production and genesis artifacts do not include it.
#[doc(hidden)]
pub fn dump_guest_coverage() {
    let mut profile = Vec::new();
    // SAFETY: Fluentbase executes a system-runtime Wasm instance sequentially. The host invokes
    // the diagnostic export only after the contract entrypoint has returned, so no guest code can
    // update LLVM's counters concurrently.
    unsafe {
        minicov::capture_coverage(&mut profile)
            .expect("guest LLVM coverage serialization must succeed");
    }
    RwasmContext.write(&profile);
    minicov::reset_coverage();

    drop(profile);
    BlockListAllocator::gc();
}
