use crate::{
    executor::RuntimeFactoryExecutor, types::ExecutionResult, RuntimeContext, RuntimeExecutor,
};
use fluentbase_types::{import_linker_v1_preview, Address, BytecodeOrHash, Bytes, B256};
use rwasm::{RwasmModule, TrapCode};
use std::{
    ops::Range,
    panic,
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc,
    },
    thread,
};

type ExecutorReply<T> = Result<T, Box<dyn std::any::Any + Send>>;

enum ExecutorCommand {
    /// Create runtime and execute entirely on this thread
    Execute {
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
        reply: SyncSender<ExecutorReply<ExecutionResult>>,
    },
    Resume {
        call_id: u32,
        return_data: Vec<u8>,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
        reply: SyncSender<ExecutorReply<ExecutionResult>>,
    },
    Warmup {
        bytecode: RwasmModule,
        hash: B256,
        address: Address,
        reply: SyncSender<ExecutorReply<()>>,
    },
    ResetCallIdCounter {
        reply: SyncSender<ExecutorReply<()>>,
    },
}

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

fn _check() {
    assert_send::<ExecutorCommand>();
}

#[derive(Clone)]
pub struct GlobalExecutor {
    tx: Arc<SyncSender<ExecutorCommand>>,
}

impl GlobalExecutor {
    pub fn new() -> Self {
        // bounded channel to avoid unbounded memory usage
        let (tx, rx): (SyncSender<ExecutorCommand>, Receiver<ExecutorCommand>) = sync_channel(4096);
        thread::Builder::new()
            .name("fluentbase-runtime-executor".to_string())
            .spawn(move || runtime_thread(rx))
            .expect("failed to start runtime executor thread");
        Self { tx: Arc::new(tx) }
    }
}

impl RuntimeExecutor for GlobalExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        let (rtx, rrx) = sync_channel(0);
        self.tx
            .send(ExecutorCommand::Execute {
                bytecode_or_hash,
                ctx,
                reply: rtx,
            })
            .expect("executor thread stopped");
        match rrx.recv().expect("executor thread stopped") {
            Ok(result) => result,
            Err(panic_payload) => panic::resume_unwind(panic_payload),
        }
    }

    fn resume(
        &mut self,
        call_id: u32,
        return_data: &[u8],
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        let (rtx, rrx) = sync_channel(0);
        self.tx
            .send(ExecutorCommand::Resume {
                call_id,
                return_data: return_data.to_vec(),
                fuel16_ptr,
                fuel_consumed,
                fuel_refunded,
                exit_code,
                reply: rtx,
            })
            .expect("executor thread stopped");
        match rrx.recv().expect("executor thread stopped") {
            Ok(result) => result,
            Err(panic_payload) => panic::resume_unwind(panic_payload),
        }
    }

    fn forget_runtime(&mut self, call_id: u32) {
        unimplemented!()
    }

    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address) {
        let (rtx, rrx) = sync_channel(0);
        self.tx
            .send(ExecutorCommand::Warmup {
                bytecode,
                hash,
                address,
                reply: rtx,
            })
            .expect("executor thread stopped");
        match rrx.recv().expect("executor thread stopped") {
            Ok(result) => result,
            Err(panic_payload) => panic::resume_unwind(panic_payload),
        }
    }

    #[cfg(feature = "wasmtime")]
    fn warmup_wasmtime(
        &mut self,
        bytecode: RwasmModule,
        _wasmtime_module: wasmtime::Module,
        hash: B256,
        address: Address,
    ) {
        // TODO(dmitry123): Add event for passing wasmtime module
        self.warmup(bytecode, hash, address)
    }

    fn reset_call_id_counter(&mut self) {
        let (rtx, rrx) = sync_channel(0);
        self.tx
            .send(ExecutorCommand::ResetCallIdCounter { reply: rtx })
            .expect("executor thread stopped");
        match rrx.recv().expect("executor thread stopped") {
            Ok(result) => result,
            Err(panic_payload) => panic::resume_unwind(panic_payload),
        }
    }

    fn memory_read(
        &mut self,
        call_id: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<(), TrapCode> {
        unimplemented!();
    }
}

fn runtime_thread(rx: Receiver<ExecutorCommand>) {
    let cores = core_affinity::get_core_ids().unwrap();
    let core_id = cores[0];
    core_affinity::set_for_current(core_id);

    let mut runtime_executor = RuntimeFactoryExecutor::new(import_linker_v1_preview());

    while let Ok(cmd) = rx.recv() {
        match cmd {
            ExecutorCommand::Execute {
                bytecode_or_hash,
                ctx,
                reply,
            } => {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    runtime_executor.execute(bytecode_or_hash, ctx)
                }));
                let _ = reply.send(result);
            }
            ExecutorCommand::Resume {
                call_id,
                return_data,
                fuel16_ptr,
                fuel_consumed,
                fuel_refunded,
                exit_code,
                reply,
            } => {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    runtime_executor.resume(
                        call_id,
                        return_data,
                        fuel16_ptr,
                        fuel_consumed,
                        fuel_refunded,
                        exit_code,
                    )
                }));
                let _ = reply.send(result);
            }
            ExecutorCommand::Warmup {
                bytecode,
                hash,
                address,
                reply,
            } => {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    runtime_executor.warmup(bytecode, hash, address)
                }));
                let _ = reply.send(result);
            }
            ExecutorCommand::ResetCallIdCounter { reply } => {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    runtime_executor.reset_call_id_counter()
                }));
                let _ = reply.send(result);
            }
        }
    }
}
