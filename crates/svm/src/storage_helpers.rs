use crate::pubkey::Pubkey;
use alloc::{rc::Rc, vec, vec::Vec};
use core::{marker::PhantomData, ops::Deref};
use fluentbase_sdk::{ExitCode, StorageAPI, B256, U256};

pub type Bytes32 = [u8; 32];

#[cfg(target_arch = "wasm32")]
#[inline(always)]
pub fn keccak256(input: &[u8]) -> B256 {
    #[link(wasm_import_module = "fluentbase_v1preview")]
    extern "C" {
        fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    }
    let mut result = B256::ZERO;
    unsafe {
        _keccak256(input.as_ptr(), input.len() as u32, result.as_mut_ptr());
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
pub fn keccak256(data: &[u8]) -> B256 {
    use keccak_hash::keccak;
    B256::new(keccak(data).0)
}

pub trait StorageSlotCalculator {
    fn storage_slot(&self, slot: u32) -> U256;
}

pub(crate) struct StorageChunksWriter<SAPI, SC> {
    pub(crate) _phantom: PhantomData<SAPI>,
    pub(crate) slot_calc: Rc<SC>,
}
impl<'a, SAPI: StorageAPI, SC: StorageSlotCalculator> FixedChunksWriter<'a, SAPI, SC>
    for StorageChunksWriter<SAPI, SC>
{
    fn slot_calc(&self) -> Rc<SC> {
        self.slot_calc.clone()
    }
}
impl<'a, SAPI: StorageAPI, SC: StorageSlotCalculator> VariableLengthDataWriter<'a, SAPI, SC>
    for StorageChunksWriter<SAPI, SC>
{
    fn set_slot_calc(&mut self, value: Rc<SC>) -> &mut Self {
        self.slot_calc = value.clone();
        self
    }

    fn slot_calc(&self) -> Rc<SC> {
        self.slot_calc.clone()
    }
}
pub trait FixedChunksWriter<'a, SAPI: StorageAPI, SC: StorageSlotCalculator> {
    fn slot_calc(&self) -> Rc<SC>;

    fn write_data_chunk_padded(
        &self,
        sapi: &mut SAPI,
        data: &[u8],
        chunk_index: u32,
        force_write: bool,
    ) -> usize {
        let start_index = chunk_index * U256::BYTES as u32;
        let end_index = start_index + U256::BYTES as u32;
        let data_tail_index = data.len() as u32;
        if start_index >= data_tail_index {
            if force_write {
                let _ = sapi.write_storage(self.slot_calc().storage_slot(chunk_index), U256::ZERO);
            }
            return 0;
        }
        let chunk =
            &data[start_index as usize..core::cmp::min(end_index, data_tail_index) as usize];
        let value = U256::from_le_slice(chunk);
        let _ = sapi.write_storage(self.slot_calc().storage_slot(chunk_index), value);
        chunk.len()
    }

    fn write_data_in_padded_chunks(
        &self,
        sapi: &mut SAPI,
        data: &[u8],
        tail_chunk_index: u32,
        force_write: bool,
    ) -> usize {
        let mut len_written = 0;
        for chunk_index in 0..=tail_chunk_index {
            len_written += self.write_data_chunk_padded(sapi, data, chunk_index, force_write)
        }
        len_written
    }

    fn read_data_chunk_padded(&self, sapi: &'a SAPI, chunk_index: u32, buf: &mut Vec<u8>) {
        let slot = self.slot_calc().storage_slot(chunk_index);
        let value = sapi.storage(&slot);
        buf.extend_from_slice(value.as_le_slice());
    }

    fn read_data_in_padded_chunks(&self, sapi: &'a SAPI, tail_chunk_index: u32, buf: &mut Vec<u8>) {
        for chunk_index in 0..=tail_chunk_index {
            self.read_data_chunk_padded(sapi, chunk_index, buf)
        }
    }
}
pub trait VariableLengthDataWriter<'a, SAPI: StorageAPI, SC: StorageSlotCalculator> {
    fn set_slot_calc(&mut self, value: Rc<SC>) -> &mut Self;
    fn slot_calc(&self) -> Rc<SC>;

    fn write_data(&self, sapi: &mut SAPI, data: &[u8]) -> usize {
        let data_len = data.len();
        if data_len <= 0 {
            return 0;
        }
        let slot = self.slot_calc().storage_slot(0);
        let _ = sapi.write_storage(slot, U256::from(data_len));
        let chunks_count = (data_len - 1) / U256::BYTES + 1;
        for chunk_index in 0..chunks_count {
            let chunk_start_index = chunk_index * U256::BYTES;
            let chunk_end_index = core::cmp::min(data_len, chunk_start_index + U256::BYTES);
            let chunk = &data[chunk_start_index..chunk_end_index];
            let value = U256::from_le_slice(chunk);
            let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
            let _ = sapi.write_storage(slot, value);
        }
        data_len
    }

    fn clear_buf_read_data(&self, sapi: &SAPI, buf: &mut Vec<u8>) -> Result<(), ExitCode> {
        buf.clear();
        self.read_data(sapi, buf)
    }

    fn read_data(&self, sapi: &SAPI, buf: &mut Vec<u8>) -> Result<(), ExitCode> {
        let slot = self.slot_calc().storage_slot(0);
        let data_len = sapi.storage(&slot);
        if data_len.status != ExitCode::Ok {
            return Err(data_len.status);
        }
        let data_len = data_len.data;
        let data_len: usize = data_len.try_into().map_err(|e| ExitCode::Err)?;
        if data_len <= 0 {
            return Ok(());
        }
        let chunks_count = (data_len - 1) / U256::BYTES + 1;
        for chunk_index in 0..chunks_count - 1 {
            let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
            let value = sapi.storage(&slot);
            let value = value.as_le_slice();
            buf.extend_from_slice(value);
        }
        let chunk_index = chunks_count - 1;
        let last_chunk_len = data_len - U256::BYTES * chunk_index;
        let slot = self.slot_calc().storage_slot(chunk_index as u32 + 1);
        let value = sapi.storage(&slot);
        let value = &value.as_le_slice()[0..last_chunk_len];
        buf.extend_from_slice(value);

        Ok(())
    }
}

pub(crate) struct IndexedHash(Bytes32);

impl Deref for IndexedHash {
    type Target = Bytes32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Into<B256> for IndexedHash {
    fn into(self) -> B256 {
        B256::from(self.0)
    }
}

impl Into<U256> for IndexedHash {
    fn into(self) -> U256 {
        U256::from_le_bytes(self.0)
    }
}

const U32_SIZE: usize = size_of::<u32>();

impl IndexedHash {
    pub(crate) fn from_hash(hash: &Bytes32) -> IndexedHash {
        IndexedHash(hash.clone())
    }

    pub(crate) fn from_data_slice(data: &[u8]) -> IndexedHash {
        let hash = keccak256(data);
        IndexedHash(hash.0)
    }

    pub(crate) fn update_with_column(mut self, column: u32) -> IndexedHash {
        if column == 0 {
            let mut preimage = vec![0u8; 32 + U32_SIZE];
            preimage[..U32_SIZE].copy_from_slice(&column.to_le_bytes());
            preimage[U32_SIZE..].copy_from_slice(self.0.as_slice());
            self.0 = keccak256(&preimage).0;
        } else {
            self.0.as_mut_slice()[..U32_SIZE].copy_from_slice(&column.to_le_bytes());
        }
        self
    }

    pub(crate) fn compute_by_column(&self, column: u32) -> IndexedHash {
        let res = IndexedHash::from_hash(&self.0);
        res.update_with_column(column)
    }

    pub(crate) fn update_with_column_index(mut self, column: u32, index: u32) -> IndexedHash {
        if index == 0 {
            let mut preimage = vec![0u8; 32 + U32_SIZE * 2];
            preimage[..U32_SIZE].copy_from_slice(&column.to_le_bytes());
            preimage[U32_SIZE..U32_SIZE * 2].copy_from_slice(&index.to_le_bytes());
            preimage[U32_SIZE * 2..].copy_from_slice(self.0.as_slice());
            self.0 = keccak256(&preimage).0;
        } else {
            self.0.as_mut_slice()[..U32_SIZE].copy_from_slice(&index.to_le_bytes());
        }
        self
    }

    pub(crate) fn compute_by_column_index(&self, column: u32, index: u32) -> IndexedHash {
        let res = IndexedHash::from_hash(&self.0);
        res.update_with_column_index(column, index)
    }

    pub(crate) fn inner(&self) -> &Bytes32 {
        &self.0
    }
}

pub trait StorageSlotHardcoded {
    fn storage_slot(hash: &Bytes32, slot: u32) -> IndexedHash {
        IndexedHash::from_hash(hash).update_with_column_index(0, slot)
    }

    fn storage_slot_raw(raw_key: &[u8], slot: u32) -> IndexedHash {
        let hash = keccak256(raw_key);
        Self::storage_slot(&hash, slot)
    }
}

pub struct ContractPubkeyHelper<'a> {
    pub pubkey: &'a Pubkey,
}

impl<'a> ContractPubkeyHelper<'a> {
    pub fn replace_pubkey(&mut self, pubkey: &'a Pubkey) {
        self.pubkey = pubkey
    }
}

impl StorageSlotHardcoded for ContractPubkeyHelper<'_> {}

impl<'a> StorageSlotCalculator for ContractPubkeyHelper<'a> {
    fn storage_slot(&self, slot: u32) -> U256 {
        <Self as StorageSlotHardcoded>::storage_slot(&self.pubkey.to_bytes(), slot).into()
    }
}
