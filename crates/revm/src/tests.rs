use crate::{
    syscall::{execute_rwasm_interruption, MemoryReaderTr},
    types::SystemInterruptionInputs,
    NextAction, RwasmContext, RwasmFrame, RwasmSpecId,
};
use alloy_primitives::{address, bytes, StorageValue, B256};
use core::error::Error;
use fluentbase_sdk::{
    byteorder::{ByteOrder, LE},
    bytes::Buf,
    calc_create_metadata_address,
    syscall::{
        SYSCALL_ID_BLOCK_HASH, SYSCALL_ID_CODE_COPY, SYSCALL_ID_METADATA_COPY,
        SYSCALL_ID_METADATA_CREATE, SYSCALL_ID_METADATA_WRITE,
    },
    Address, Bytes, SyscallInvocationParams, PRECOMPILE_WASM_RUNTIME, STATE_MAIN, U256,
};
use revm::{
    bytecode::{ownable_account::OwnableAccountBytecode, Bytecode},
    context::{BlockEnv, CfgEnv, ContextError, ContextTr, Host, JournalTr, TxEnv},
    database::{DBErrorMarker, InMemoryDB},
    inspector::NoOpInspector,
    interpreter::{Gas, InstructionResult},
    state::AccountInfo,
    Database,
};
use rwasm::TrapCode;
use std::{fmt, vec, vec::Vec};

/// Test-only memory reader that serves bytes from an in-memory buffer.
pub(crate) struct ForwardInputMemoryReader(Bytes);

impl MemoryReaderTr for ForwardInputMemoryReader {
    fn memory_read(&self, _call_id: u32, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        self.0
            .get(offset..offset + buffer.len())
            .ok_or(TrapCode::MemoryOutOfBounds)?
            .copy_to_slice(buffer);
        Ok(())
    }
}

#[cfg(test)]
mod code_copy_tests {
    use super::*;

    /// Helper function to test code_copy syscall
    /// Returns (output_data, gas_used)
    fn test_code_copy_helper(
        bytecode: Bytes,
        code_offset: u64,
        code_length: u64,
        initial_gas: u64,
    ) -> (Bytes, u64) {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        let mut frame = RwasmFrame::default();

        // === Setup: Create target account with known bytecode ===
        let target_address = Address::from([0x42; 20]);

        {
            let mut account = ctx
                .journal_mut()
                .load_account_with_code_mut(target_address)
                .unwrap();
            account.set_code_and_hash_slow(Bytecode::new_raw(bytecode.clone()));
            account.set_balance(U256::ONE); // Non-empty account
        }

        // === Prepare syscall input ===
        let mut syscall_input = vec![0u8; 20 + 8 + 8];
        syscall_input[0..20].copy_from_slice(target_address.as_slice());
        syscall_input[20..28].copy_from_slice(&code_offset.to_le_bytes());
        syscall_input[28..36].copy_from_slice(&code_length.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_CODE_COPY,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(initial_gas),
        };

        // === Execute: Call the syscall ===
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );

        // === Extract results ===
        let returned_data = frame.interrupted_outcome.as_ref().unwrap();
        let output_data = returned_data.result.as_ref().unwrap().output.clone();

        // Get gas_used from the interruption result directly.
        let gas_used = returned_data.result.as_ref().unwrap().gas.spent();

        (output_data, gas_used)
    }

    fn expected_gas(length: usize, is_cold: bool) -> u64 {
        // Base cost depends on warm/cold access.
        // After EIP-2929 (BERLIN fork):
        // - Cold access: 2600 gas (first access to account)
        // - Warm access: 100 gas (subsequent accesses)
        let base_gas = if is_cold {
            2600 // COLD_ACCOUNT_ACCESS_COST
        } else {
            100 // WARM_STORAGE_READ_COST
        };

        // Copy cost: 3 gas per 32-byte word.
        // Formula: 3 * ceil(length / 32)
        let words = (length + 31) / 32; // ceil(length / 32)
        let copy_gas = words as u64 * 3;

        base_gas + copy_gas
    }

    #[test]
    fn test_code_copy_basic_slice() {
        // Test: Basic slice from middle of bytecode.
        let bytecode = Bytes::from(vec![
            0x60, 0x80, 0x60, 0x40, 0x52, 0x33, 0x90, 0x81, 0x01, 0x02,
        ]); // 10 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 3, 4, 10_000_000);

        assert_eq!(output.len(), 4, "Should return 4 bytes");
        assert_eq!(
            &output[..],
            &bytecode[3..7],
            "Should return bytes at offset 3-6"
        );
        assert_eq!(gas, expected_gas(4, false));
    }

    #[test]
    fn test_code_copy_request_more_than_available() {
        // Test: Request more bytes than available (no padding in Wasm)
        let bytecode = Bytes::from(vec![0xAA, 0xBB, 0xCC]); // Only 3 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 1, 10, 10_000_000);

        assert_eq!(output.len(), 10, "Should return only 2 available bytes");
        assert_eq!(
            &output[..],
            &[0xBB, 0xCC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(gas, expected_gas(10, false));
    }

    #[test]
    fn test_code_copy_offset_beyond_bytecode() {
        // Test: Offset completely beyond bytecode length.
        let bytecode = Bytes::from(vec![0xAA, 0xBB, 0xCC]); // 3 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 100, 5, 10_000_000);
        // Should return empty bytes.
        assert_eq!(
            output.len(),
            5,
            "Should return empty bytes when offset > bytecode.len()"
        );
        assert_eq!(gas, expected_gas(5, false));
    }

    #[test]
    fn test_code_copy_empty_bytecode() {
        // Test: Copy from account with empty/no bytecode.
        let bytecode = Bytes::new();

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 0, 10, 10_000_000);

        assert_eq!(
            output.len(),
            10,
            "Should return empty bytes for empty bytecode"
        );
        assert_eq!(
            &output[..],
            &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );

        assert_eq!(gas, expected_gas(10, false));
    }

    #[test]
    fn test_code_copy_gas_calculation() {
        // Test: Verify gas is calculated correctly according to EVM rules.
        let bytecode = Bytes::from(vec![0xFF; 200]); // 100 bytes

        let (output, gas) = test_code_copy_helper(bytecode.clone(), 0, 200, 10_000_000);

        assert_eq!(output.len(), 200);
        assert_eq!(output[..], bytecode);
        assert_eq!(gas, expected_gas(200, false));
    }
}

#[cfg(test)]
mod metadata_write_tests {
    use super::*;

    #[test]
    fn test_metadata_write_truncates_existing_data() {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        // === Setup: Create frame with owner address ===
        let owner_address = PRECOMPILE_WASM_RUNTIME;
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(owner_address);

        // === Step 1: Create an account with initial metadata ===
        let test_address = Address::from_slice(&[0x42; 20]);
        let initial_metadata = Bytes::from(&[0xFF; 100]);

        let _ = ctx.journal_mut().load_account(test_address);

        // Pre-create the ownable account with initial metadata.
        ctx.journal_mut().set_code(
            test_address,
            Bytecode::OwnableAccount(OwnableAccountBytecode::new(
                owner_address,
                initial_metadata.clone(),
            )),
        );

        let new_data = vec![0x11, 0x22, 0x33, 0x44];
        let offset = 2u32;

        // Prepare syscall input: address (20 bytes) + offset (4 bytes) + data.
        let mut syscall_input = Vec::new();
        syscall_input.extend_from_slice(&test_address.0[..]);
        syscall_input.extend_from_slice(&offset.to_le_bytes());
        syscall_input.extend_from_slice(&new_data);
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_METADATA_WRITE,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            fuel_limit: 1_000_000,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(1_000_000),
        };

        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );

        let acc = ctx
            .journal_mut()
            .load_account_with_code(test_address)
            .unwrap();
        match &acc.info.code {
            Some(Bytecode::OwnableAccount(ownable)) => {
                assert_eq!(ownable.metadata[..], new_data);
            }
            _ => panic!("Expected OwnableAccount bytecode"),
        }
    }

    #[test]
    fn test_metadata_create() {
        // === Setup: Initialize context and database ===
        let db = InMemoryDB::default();
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        // === Setup: Create frame with owner address ===
        let owner_address = PRECOMPILE_WASM_RUNTIME;
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(owner_address);

        // === Prepare: Calculate derived address and ensure it's not empty ===
        let salt = U256::from(123456789u64);
        let metadata = Bytes::from(vec![0x01, 0x02, 0x03]);

        let derived_address = calc_create_metadata_address(&owner_address, &salt);

        // Preload the account to avoid empty account check.
        // (In real scenario, this would be done by previous transactions)
        {
            let mut account = ctx
                .journal_mut()
                .load_account_with_code_mut(derived_address)
                .unwrap();
            account.set_balance(U256::ONE);
        }

        // === Execute: Prepare syscall input (salt + metadata) ===
        let mut syscall_input = Vec::with_capacity(32 + metadata.len());
        syscall_input.extend_from_slice(&salt.to_be_bytes::<32>());
        syscall_input.extend_from_slice(&metadata);
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_METADATA_CREATE,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(1_000_000), // Sufficient gas for the operation
        };

        // === Execute: Call the syscall ===
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        assert!(
            result.is_ok(),
            "Syscall execution failed: {:?}",
            result.err()
        );
        assert!(
            matches!(result.unwrap(), NextAction::InterruptionResult),
            "Expected InterruptionResult"
        );

        // === Verify: Check the created account ===
        let created_account = ctx
            .journal_mut()
            .load_account_with_code(derived_address)
            .expect("Failed to load created account");

        match &created_account.info.code {
            Some(Bytecode::OwnableAccount(ownable)) => {
                assert_eq!(
                    ownable.owner_address, owner_address,
                    "Owner address mismatch"
                );
                assert_eq!(ownable.metadata, metadata, "Metadata mismatch");
            }
            other => panic!("Expected OwnableAccount bytecode, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod block_hash_tests {
    use super::*;

    #[derive(Debug, Clone)]
    pub(super) struct MockDbError(pub String);

    impl fmt::Display for MockDbError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl Error for MockDbError {}
    impl DBErrorMarker for MockDbError {}

    struct FailingMockDatabase;

    impl Database for FailingMockDatabase {
        type Error = MockDbError;

        fn basic(&mut self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
            Ok(None)
        }

        fn code_by_hash(&mut self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
            Ok(Bytecode::default())
        }

        fn storage(
            &mut self,
            _address: Address,
            _index: U256,
        ) -> Result<StorageValue, Self::Error> {
            Ok(StorageValue::default())
        }

        fn block_hash(&mut self, _number: u64) -> Result<B256, Self::Error> {
            Err(MockDbError("Database I/O error".to_string()))
        }
    }

    /// Helper function to test block_hash syscall
    fn test_block_hash_helper<DB: Database>(
        db: DB,
        current_block: u64,
        requested_block: u64,
    ) -> Result<Bytes, ContextError<DB::Error>> {
        let mut ctx: RwasmContext<DB> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        ctx.block.number = U256::from(current_block);

        let mut frame = RwasmFrame::default();

        let mut syscall_input = vec![0u8; 8];
        syscall_input[0..8].copy_from_slice(&requested_block.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let syscall_params = SyscallInvocationParams {
            code_hash: SYSCALL_ID_BLOCK_HASH,
            input: 0..mr.0.len(),
            state: STATE_MAIN,
            ..Default::default()
        };

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params,
            gas: Gas::new(10_000_000),
        };

        execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )?;

        Ok(frame
            .interrupted_outcome
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .output
            .clone())
    }

    #[test]
    fn test_block_hash_database_error_is_propagated() {
        // This test verifies that database errors are propagated through execute_rwasm_interruption.
        // Instead of being silently converted to zero hash.

        let db = FailingMockDatabase;
        let mut ctx: RwasmContext<FailingMockDatabase> = RwasmContext::new(db, RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        ctx.block.number = U256::from(1000);

        let mut frame = RwasmFrame::default();

        // Request a valid block (within the last 256 blocks)
        let requested_block: u64 = 900;
        let mut syscall_input = vec![0u8; 8];
        syscall_input[0..8].copy_from_slice(&requested_block.to_le_bytes());
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_BLOCK_HASH,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };

        // Execute the syscall - should return Err, not Ok.
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        );

        // Assert that database error was propagated.
        assert!(
            result.is_err(),
            "Database error must be propagated, not hidden"
        );
        println!("result: {:?}", result);
    }

    #[test]
    fn test_block_hash_database_error_propagation() {
        let db = FailingMockDatabase;
        let result = test_block_hash_helper(db, 1000, 900);

        assert!(
            result.is_err(),
            "Expected database error to be propagated, but got Ok"
        );

        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(
            error_msg.contains("Database I/O error") || error_msg.contains("MockDbError"),
            "Error should contain database error message, got: {}",
            error_msg
        );
    }

    #[test]
    fn test_block_hash_out_of_range_returns_zero() {
        let db = InMemoryDB::default();
        let output =
            test_block_hash_helper(db, 1000, 1001).expect("Should succeed for out of range block");

        assert_eq!(output.len(), 32, "Should return 32 bytes");
        assert_eq!(
            &output[..],
            B256::ZERO.as_slice(),
            "Should return zero hash for future block"
        );
    }

    #[test]
    fn test_block_hash_too_old_returns_zero() {
        let db = InMemoryDB::default();
        let output =
            test_block_hash_helper(db, 1000, 743).expect("Should succeed for too old block");

        assert_eq!(
            &output[..],
            B256::ZERO.as_slice(),
            "Should return zero hash for block older than 256"
        );
    }

    #[test]
    fn test_metadata_copy_out_of_bounds() {
        let mut ctx: RwasmContext<InMemoryDB> =
            RwasmContext::new(InMemoryDB::default(), RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(Address::ZERO);

        const ADDRESS: Address = address!("1111111111111111111111111111111111111111");
        _ = ctx.load_account_delegated(ADDRESS).unwrap();
        ctx.journal_mut().set_code(
            ADDRESS,
            Bytecode::new_ownable_account(Address::ZERO, bytes!("112233445566")),
        );

        let mut syscall_input = vec![0u8; 28];
        syscall_input[0..20].copy_from_slice(ADDRESS.as_slice());
        LE::write_u32(&mut syscall_input[20..24], 100); // offset
        LE::write_u32(&mut syscall_input[24..28], 0); // length
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_METADATA_COPY,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };
        execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )
        .unwrap();
        let output = frame
            .interrupted_outcome
            .as_ref()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .output
            .clone();
        assert_eq!(output, Bytes::new());
    }

    #[test]
    fn test_metadata_write_static_context() {
        let mut ctx: RwasmContext<InMemoryDB> =
            RwasmContext::new(InMemoryDB::default(), RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();
        let mut frame = RwasmFrame::default();
        frame.interpreter.input.account_owner = Some(Address::ZERO);
        frame.interpreter.runtime_flag.is_static = true;

        const ADDRESS: Address = address!("1111111111111111111111111111111111111111");
        _ = ctx.load_account_delegated(ADDRESS).unwrap();

        let mut syscall_input = vec![0u8; 24];
        syscall_input[0..20].copy_from_slice(ADDRESS.as_slice());
        LE::write_u32(&mut syscall_input[20..24], 0); // _offset
        let mr = ForwardInputMemoryReader(syscall_input.into());

        let interruption_inputs = SystemInterruptionInputs {
            call_id: 0,
            syscall_params: SyscallInvocationParams {
                code_hash: SYSCALL_ID_METADATA_WRITE,
                input: 0..mr.0.len(),
                state: STATE_MAIN,
                ..Default::default()
            },
            gas: Gas::new(10_000_000),
        };
        let result = execute_rwasm_interruption::<_, NoOpInspector>(
            &mut frame,
            None,
            &mut ctx,
            interruption_inputs,
            mr,
        )
        .unwrap();
        assert_eq!(
            result
                .into_interpreter_action()
                .instruction_result()
                .unwrap(),
            InstructionResult::StateChangeDuringStaticCall
        );
    }
}
