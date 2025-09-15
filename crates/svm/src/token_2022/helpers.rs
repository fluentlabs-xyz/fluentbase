use crate::account::{Account, AccountSharedData, ReadableAccount};
use crate::error::SvmError;
use crate::helpers::{storage_read_account_data, storage_write_account_data};
use crate::token_2022;
use alloc::vec::Vec;
use fluentbase_sdk::debug_log_ext;
use fluentbase_types::{SharedAPI, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME};
use hashbrown::{HashMap, HashSet};
use solana_account_info::{AccountInfo, IntoAccountInfo};
use solana_instruction::AccountMeta;
use solana_program_error::ProgramError;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;

pub fn next_item<'a, 'b, T, I: Iterator<Item = &'a T>>(
    iter: &mut I,
) -> Result<I::Item, ProgramError> {
    iter.next().ok_or(ProgramError::NotEnoughAccountKeys)
}

pub fn account_info_from_meta_and_account<'a>(
    account_meta: &'a AccountMeta,
    account: &'a mut Account,
) -> AccountInfo<'a> {
    AccountInfo::new(
        &account_meta.pubkey,
        account_meta.is_signer,
        account_meta.is_writable,
        &mut account.lamports,
        &mut account.data,
        &account.owner,
        account.executable,
        account.rent_epoch,
    )
}

pub fn account_from_account_info(account_info: &AccountInfo) -> Account {
    Account {
        lamports: account_info.lamports(),
        data: account_info.data.borrow().to_vec(),
        owner: account_info.owner.clone(),
        executable: account_info.executable,
        rent_epoch: account_info.rent_epoch,
    }
}

pub fn account_meta_from_account_info(account_info: &AccountInfo) -> AccountMeta {
    if account_info.is_writable {
        AccountMeta::new(account_info.key.clone(), account_info.is_signer)
    } else {
        AccountMeta::new_readonly(account_info.key.clone(), account_info.is_signer)
    }
}

pub fn reconstruct_accounts<SDK: SharedAPI, const DEFAULT_NON_EXISTENT: bool>(
    sdk: &SDK,
    account_metas: &[AccountMeta],
) -> Result<Vec<Account>, SvmError> {
    let mut accounts = Vec::<Account>::with_capacity(account_metas.len());
    let mut pks_reconstructed = HashMap::<Pubkey, usize>::with_capacity(account_metas.len());
    for account_meta in account_metas {
        if let Some(idx) = pks_reconstructed.get(&account_meta.pubkey) {
            debug_log_ext!(
                "account_meta.pubkey {:x?} idx {}",
                account_meta.pubkey.to_bytes(),
                idx
            );
            accounts.push(accounts[*idx].clone());
            continue;
        }
        pks_reconstructed.insert(account_meta.pubkey, accounts.len());
        let account_data = storage_read_account_data(
            sdk,
            &account_meta.pubkey,
            Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
        );
        let account_data = if let Ok(account_data) = account_data {
            account_data
        } else if DEFAULT_NON_EXISTENT {
            AccountSharedData::new(
                0,
                token_2022::state::Account::get_packed_len(),
                &token_2022::lib::id(),
            )
        } else {
            account_data?
        };
        debug_log_ext!(
            "pk {:x?} account_data {:x?} data {:x?}",
            account_meta.pubkey.to_bytes(),
            account_data,
            account_data.data(),
        );
        accounts.push(account_data.into());
    }
    Ok(accounts)
}

pub fn reconstruct_account_infos<'a>(
    account_metas: &'a [AccountMeta],
    accounts: &'a mut [Account],
) -> Result<Vec<AccountInfo<'a>>, SvmError> {
    let mut account_infos: Vec<AccountInfo> = Vec::with_capacity(account_metas.len());
    let mut pks_reconstructed = HashMap::<Pubkey, usize>::with_capacity(account_metas.len());
    for (account_meta, account) in account_metas.iter().zip(accounts.iter_mut()) {
        let account_info = if let Some(idx) = pks_reconstructed.get(&account_meta.pubkey) {
            debug_log_ext!(
                "account_meta.pubkey {:x?} idx {}",
                account_meta.pubkey.to_bytes(),
                idx
            );
            account_infos[*idx].clone()
        } else {
            pks_reconstructed.insert(account_meta.pubkey, account_infos.len());
            account_info_from_meta_and_account(account_meta, account)
        };
        account_infos.push(account_info);
    }
    Ok(account_infos)
}

pub fn normalize_account_metas<'a>(account_metas: &'a mut [AccountMeta]) {
    let mut account_metas_normed =
        HashMap::<Pubkey, AccountMeta>::with_capacity(account_metas.len());
    for account_meta in account_metas.iter_mut() {
        if let Some(v) = account_metas_normed.get_mut(&account_meta.pubkey) {
            v.is_signer |= account_meta.is_signer;
            v.is_writable |= account_meta.is_writable;
        } else {
            account_metas_normed.insert(account_meta.pubkey, account_meta.clone());
        }
    }
    for account_meta in account_metas {
        let normed = account_metas_normed.get(&account_meta.pubkey).unwrap();
        account_meta.is_writable = normed.is_writable;
        account_meta.is_signer = normed.is_signer;
    }
}
pub fn flush_accounts<SDK: SharedAPI, const SKIP_REPEATS: bool>(
    sdk: &mut SDK,
    accounts_datas: &[(&AccountMeta, &Account)],
) -> Result<(), ProgramError> {
    let mut flushed_pks = HashSet::<Pubkey>::default();
    for (account_meta, account) in accounts_datas {
        let pk = &account_meta.pubkey;
        if SKIP_REPEATS && !flushed_pks.insert(pk.clone()) {
            continue;
        }
        let account_data: AccountSharedData = (*account).clone().into();
        storage_write_account_data(
            sdk,
            pk,
            &account_data,
            Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
        )
        .expect("result account data not written");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::account::{Account, AccountSharedData};
    use crate::token_2022::helpers::reconstruct_account_infos;
    use solana_instruction::AccountMeta;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_reconstruct_account_infos() {
        let pk1 = Pubkey::new_unique();
        let pk1_owner = Pubkey::new_unique();
        let pk2 = pk1.clone();
        let pk2_owner = Pubkey::new_unique();

        let am1 = AccountMeta::new(pk1, false);
        let ad1 = AccountSharedData::new(1, 10, &pk1_owner);
        let ac1: Account = ad1.into();

        let am2 = AccountMeta::new(pk2, false);
        let ad2 = AccountSharedData::new(1, 10, &pk2_owner);
        let ac2: Account = ad2.into();

        let account_metas = &[am1, am2];
        let accounts = &mut [ac1, ac2];
        let account_infos = reconstruct_account_infos(account_metas, accounts).unwrap();

        assert_eq!(
            account_infos[0].data.borrow().iter().as_slice(),
            account_infos[1].data.borrow().iter().as_slice(),
        );

        let mut data_mut = account_infos[0].data.borrow_mut();
        data_mut.as_mut()[3] = 2;
        drop(data_mut);

        assert_eq!(
            account_infos[0].data.borrow().iter().as_slice(),
            account_infos[1].data.borrow().iter().as_slice(),
        );
        assert_eq!(*account_infos[0].data.borrow().get(3).unwrap(), 2);
        assert_eq!(accounts[0].data[3], 2);
    }
}
