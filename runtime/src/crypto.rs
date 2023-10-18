use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub(crate) fn crypto_keccak(mut caller: Caller<'_, RuntimeContext>) -> Result<(), Trap> {
    Ok(())
}
