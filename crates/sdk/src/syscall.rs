use core::mem::take;
use fluentbase_types::{
    contracts::SYSCALL_ID_COLD_STORAGE_READ,
    Account,
    Address,
    Bytes,
    DelayedInvocationParams,
    ExitCode,
    Fuel,
    NativeAPI,
    SovereignAPI,
    B256,
    STATE_MAIN,
    U256,
};

#[derive(Clone, Debug)]
enum NextAction {
    ExecutionResult(Bytes, i32),
    NestedCall(u32, DelayedInvocationParams),
}

impl NextAction {
    fn from_exit_code(exit_code: ExitCode) -> Self {
        Self::ExecutionResult(Bytes::default(), exit_code.into_i32())
    }
}

fn _syscall_cold_storage_read<SDK: SovereignAPI>(
    sdk: &mut SDK,
    self_account: &Address,
    params: DelayedInvocationParams,
) -> NextAction {
    // make sure default fields are set to 0
    if !params.code_hash.is_zero() {
        return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
    } else if params.state != STATE_MAIN {
        return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
    }

    // make sure input is 32 bytes len
    if params.input.len() != 32 {
        return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
    }

    // read value from storage
    let slot = U256::from_le_slice(params.input.as_ref());
    let (value, _) = sdk.storage(&self_account, &slot);

    // return value as bytes with success exit code
    NextAction::ExecutionResult(value.to_le_bytes::<32>().into(), ExitCode::Ok.into_i32())
}

fn _process_exec_params<SDK: SovereignAPI>(
    sdk: &mut SDK,
    address: &Address,
    exit_code: i32,
) -> NextAction {
    // if the exit code is non-positive (stands for termination), then execution is finished
    if exit_code <= 0 {
        return NextAction::ExecutionResult(sdk.native_sdk().return_data(), exit_code);
    }

    // otherwise, exit code is a "call_id" that identifies saved context
    let call_id = exit_code as u32;

    // try to parse execution params, if it's not possible then return an error
    let exec_params = sdk.native_sdk().return_data();
    let Some(params) = DelayedInvocationParams::from_slice(exec_params.as_ref()) else {
        return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
    };

    if params.address == SYSCALL_ID_COLD_STORAGE_READ {
        _syscall_cold_storage_read(sdk, &address, params)
    } else {
        NextAction::NestedCall(call_id, params)
    }
}

fn _syscall_exec<SDK: SovereignAPI>(
    sdk: &mut SDK,
    address: &Address,
    params: &DelayedInvocationParams,
    return_call_id: u32,
) -> NextAction {
    // we don't do all these checks for root level
    // because root level is trusted and can do any calls
    if return_call_id > 0 {
        // only main state can be forwarded to the other contract as a nested call,
        // other states can be only used by root
        if params.state != STATE_MAIN {
            return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
        }
        // for non-delegate call, make sure the provided hash matches to the target code hash,
        // if it doesn't match then it's context rewrite that is prohibited by non-root
        let is_delegate_call = address == &params.address;
        if !is_delegate_call {
            let (target_account, _) = sdk.account(&params.address);
            if target_account.rwasm_code_hash != params.code_hash {
                return NextAction::from_exit_code(ExitCode::MalformedSyscallParams);
            }
        }
    }

    // warmup bytecode before execution,
    // it's a technical limitation we're having right now,
    // planning to solve it in the future
    #[cfg(feature = "std")]
    {
        use fluentbase_runtime::Runtime;
        let bytecode = sdk.preimage(&params.code_hash).unwrap_or_default();
        Runtime::warmup_bytecode(params.code_hash, bytecode);
    }

    // execute smart contract
    let exit_code = sdk.native_sdk().exec(
        &params.code_hash,
        &params.address,
        params.input.as_ref(),
        params.fuel_limit,
        params.state,
    );

    _process_exec_params(sdk, address, exit_code)
}

fn _syscall_resume<SDK: SovereignAPI>(
    sdk: &mut SDK,
    address: &Address,
    call_id: u32,
    return_data: &[u8],
    exit_code: i32,
) -> NextAction {
    let exit_code = sdk.native_sdk().resume(call_id, return_data, exit_code);
    _process_exec_params(sdk, address, exit_code)
}

#[derive(Debug)]
enum Frame {
    Execute(DelayedInvocationParams, u32),
    Resume(Address, u32, Bytes, i32),
}

impl Frame {
    fn call_id(&self) -> u32 {
        match self {
            Frame::Execute(_, call_id) => *call_id,
            Frame::Resume(_, call_id, _, _) => *call_id,
        }
    }
}

pub fn execute_rwasm_smart_contract<SDK: SovereignAPI>(
    sdk: &mut SDK,
    account: &Account,
    fuel: &mut Fuel,
    input: &[u8],
    state: u32,
) -> (Bytes, ExitCode) {
    let mut call_stack: Vec<Frame> = vec![Frame::Execute(
        DelayedInvocationParams {
            code_hash: account.rwasm_code_hash,
            address: account.address,
            input: Bytes::copy_from_slice(input),
            fuel_limit: fuel.remaining(),
            state,
        },
        0,
    )];

    let mut stack_frame = call_stack.last_mut().unwrap();
    loop {
        let next_action = match stack_frame {
            Frame::Execute(params, return_call_id) => {
                _syscall_exec(sdk, &params.address, params, *return_call_id)
            }
            Frame::Resume(address, return_call_id, return_data, exit_code) => _syscall_resume(
                sdk,
                address,
                *return_call_id,
                return_data.as_ref(),
                *exit_code,
            ),
        };

        match next_action {
            NextAction::ExecutionResult(return_data, exit_code) => {
                let _resumable_frame = call_stack.pop().unwrap();
                if call_stack.is_empty() {
                    return (return_data, ExitCode::from(exit_code));
                }
                match call_stack.last_mut().unwrap() {
                    Frame::Resume(_, _, return_data_result, exit_code_result) => {
                        *return_data_result = return_data;
                        *exit_code_result = exit_code;
                    }
                    _ => unreachable!(),
                }
                stack_frame = call_stack.last_mut().unwrap();
            }
            NextAction::NestedCall(call_id, params) => {
                let last_frame = call_stack.last_mut().unwrap();
                match last_frame {
                    Frame::Execute(params, _) => {
                        *last_frame =
                            Frame::Resume(params.address, call_id, Bytes::default(), i32::MIN);
                    }
                    Frame::Resume(_, call_id_result, _, _) => {
                        *call_id_result = call_id;
                    }
                }
                call_stack.push(Frame::Execute(params, call_id));
                stack_frame = call_stack.last_mut().unwrap();
            }
        }
    }
}
