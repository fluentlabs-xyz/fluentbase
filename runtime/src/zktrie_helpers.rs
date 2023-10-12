use fluentbase_rwasm::common::Trap;
use zktrie::{AccountData, ACCOUNTSIZE, FIELDSIZE};

pub(crate) fn account_data_from_bytes(data: &[u8]) -> Result<AccountData, Trap> {
    if data.len() != ACCOUNTSIZE {
        return Err(Trap::new("account data bad len"));
    }
    let mut ad: AccountData = Default::default();
    for (i, b) in data.iter().enumerate() {
        ad[i / FIELDSIZE][i % FIELDSIZE] = *b;
    }

    Ok(ad)
}
