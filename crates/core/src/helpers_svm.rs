use crate::fvm::types::WasmRelayer;
use alloc::vec::Vec;
use fuel_core_executor::executor::{
    BlockExecutor,
    ExecutionData,
    ExecutionOptions,
    TxStorageTransaction,
};
use fuel_core_storage::{
    column::Column,
    kv_store::{KeyValueInspect, KeyValueMutate, WriteOperation},
    structured_storage::StructuredStorage,
    transactional::{Changes, ConflictPolicy, InMemoryTransaction, IntoTransaction},
};
use fuel_core_types::{
    blockchain::header::PartialBlockHeader,
    fuel_tx::{Cacheable, ConsensusParameters, ContractId, Receipt, Word},
    fuel_vm::{
        checked_transaction::{Checked, IntoChecked},
        interpreter::{CheckedMetadata, ExecutableTransaction, MemoryInstance},
        ProgramState,
    },
    services::executor::Result,
};
use fuel_core_types::services::executor::Error;
use crate::helpers_fvm::FvmTransactResult;

extern crate byteorder;
extern crate solana_rbpf;

use solana_rbpf::{
    aligned_memory::AlignedMemory,
    ebpf::HOST_ALIGN,
    memory_region::MemoryCowCallback,
    assembler::assemble,
    //declare_builtin_function, // [todo:rgds]
    ebpf,
    elf::Executable,
    error::{EbpfError, ProgramResult},
    memory_region::{MemoryMapping, MemoryRegion},
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry, SBPFVersion},
    static_analysis::Analysis,
    syscalls,
    verifier::RequisiteVerifier,
    vm::{Config, ContextObject, TestContextObject},
};
use std::{fs::File, io::Read, sync::Arc};

macro_rules! create_vm {
    ($vm_name:ident, $verified_executable:expr, $context_object:expr, $stack:ident,
    $heap:ident, $additional_regions:expr, $cow_cb:expr) => {
        // here we have error r/o heap on wasm:
        //let mut $heap = solana_rbpf::aligned_memory::AlignedMemory::with_capacity(0);
        // fix (do not use with_capacity() more):
        let mut $heap = solana_rbpf::aligned_memory::AlignedMemory::zero_filled(
            1024*1024,
        );
        let mut $stack = solana_rbpf::aligned_memory::AlignedMemory::zero_filled(
            $verified_executable.get_config().stack_size(),
        );
        let stack_len = $stack.len();
        let memory_mapping = create_memory_mapping(
            $verified_executable,
            &mut $stack,
            &mut $heap,
            $additional_regions,
            $cow_cb,
        )
        .unwrap();
        let mut $vm_name = solana_rbpf::vm::EbpfVm::new(
            $verified_executable.get_loader().clone(),
            $verified_executable.get_sbpf_version(),
            $context_object,
            memory_mapping,
            stack_len,
        );
    };
}


pub fn create_memory_mapping<'a, C: ContextObject>(
    executable: &'a Executable<C>,
    stack: &'a mut AlignedMemory<{ HOST_ALIGN }>,
    heap: &'a mut AlignedMemory<{ HOST_ALIGN }>,
    additional_regions: Vec<MemoryRegion>,
    cow_cb: Option<MemoryCowCallback>,
) -> core::result::Result<MemoryMapping<'a>, EbpfError> {
    let config = executable.get_config();
    let sbpf_version = executable.get_sbpf_version();

    println!("Creating memory mapping:");
    println!("Stack size: {}", stack.len());
    println!("Heap size: {}", heap.len());

    for region in &additional_regions {
        // println!("Additional region size: {}", region.len());
        // assert!(region.len() > 0, "Region size must be greater than zero");
    }

    let regions: Vec<MemoryRegion> = vec![
        executable.get_ro_region(),
        MemoryRegion::new_writable_gapped(
            stack.as_slice_mut(),
            ebpf::MM_STACK_START,
            if !sbpf_version.dynamic_stack_frames() && config.enable_stack_frame_gaps {
                config.stack_frame_size as u64
            } else {
                0
            },
        ),
        MemoryRegion::new_writable(heap.as_slice_mut(), ebpf::MM_HEAP_START),
    ]
        .into_iter()
        .chain(additional_regions.into_iter())
        .collect();

    println!("Memory regions created: {:?}", regions);
    // Program code starts at `0x100000000`
    // Stack data starts at `0x200000000`
    // Heap data starts at `0x300000000`
    // Program input parameters start at `0x400000000`
    // Solana offers 4KB of stack frame space and 32KB of heap space by default

    Ok(if let Some(cow_cb) = cow_cb {
        MemoryMapping::new_with_cow(regions, cow_cb, config, sbpf_version)?
    } else {
        MemoryMapping::new(regions, config, sbpf_version)?
    })
}

const INSTRUCTION_METER_BUDGET: u64 = 1024*1024;

macro_rules! test_interpreter_and_jit {
    (register, $function_registry:expr, $location:expr => $syscall_function:expr) => {
        $function_registry
            .register_function_hashed($location.as_bytes(), $syscall_function)
            .unwrap();
    };
    ($executable:expr, $mem:tt, $context_object:expr, $expected_result:expr $(,)?) => {
        let expected_instruction_count = $context_object.get_remaining();
        #[allow(unused_mut)]
        let mut context_object = $context_object;
        let expected_result = format!("{:?}", $expected_result);
        if !expected_result.contains("ExceededMaxInstructions") {
            context_object.remaining = INSTRUCTION_METER_BUDGET;
        }
        $executable.verify::<RequisiteVerifier>().unwrap();
        let (instruction_count_interpreter, interpreter_final_pc, _tracer_interpreter) = {
            let mut mem = $mem;
            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);
            let mut context_object = context_object.clone();
            create_vm!(
                vm,
                &$executable,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            let (instruction_count_interpreter, result) = vm.execute_program(&$executable, true);
            assert_eq!(
                format!("{:?}", result),
                expected_result,
                "Unexpected result for Interpreter"
            );
            (
                instruction_count_interpreter,
                vm.registers[11],
                vm.context_object_pointer.clone(),
            )
        };
        #[cfg(all(feature = "jit", not(target_os = "windows"), target_arch = "x86_64"))]
        {
            #[allow(unused_mut)]
            let compilation_result = $executable.jit_compile();
            let mut mem = $mem;
            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);
            create_vm!(
                vm,
                &$executable,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            match compilation_result {
                Err(err) => assert_eq!(
                    format!("{:?}", err),
                    expected_result,
                    "Unexpected result for JIT compilation"
                ),
                Ok(()) => {
                    let (instruction_count_jit, result) = vm.execute_program(&$executable, false);
                    let tracer_jit = &vm.context_object_pointer;
                    if !TestContextObject::compare_trace_log(&_tracer_interpreter, tracer_jit) {
                        let analysis = Analysis::from_executable(&$executable).unwrap();
                        let stdout = std::io::stdout();
                        analysis
                            .disassemble_trace_log(
                                &mut stdout.lock(),
                                &_tracer_interpreter.trace_log,
                            )
                            .unwrap();
                        analysis
                            .disassemble_trace_log(&mut stdout.lock(), &tracer_jit.trace_log)
                            .unwrap();
                        panic!();
                    }
                    assert_eq!(
                        format!("{:?}", result),
                        expected_result,
                        "Unexpected result for JIT"
                    );
                    assert_eq!(
                        instruction_count_interpreter, instruction_count_jit,
                        "Interpreter and JIT instruction meter diverged",
                    );
                    assert_eq!(
                        interpreter_final_pc, vm.registers[11],
                        "Interpreter and JIT instruction final PC diverged",
                    );
                }
            }
        }
        if $executable.get_config().enable_instruction_meter {
            assert_eq!(
                instruction_count_interpreter, expected_instruction_count,
                "Instruction meter did not consume expected amount"
            );
        }
    };
}

macro_rules! test_interpreter_and_jit_asm {
    ($source:tt, $config:expr, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            let mut config = $config;
            config.enable_instruction_tracing = true;
            let mut function_registry = FunctionRegistry::<BuiltinFunction<TestContextObject>>::default();
            $(test_interpreter_and_jit!(register, function_registry, $location => $syscall_function);)*
            let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
            let mut executable = assemble($source, loader).unwrap();
            test_interpreter_and_jit!(executable, $mem, $context_object, $expected_result);
        }
    };
    ($source:tt, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            test_interpreter_and_jit_asm!($source, Config::default(), $mem, ($($location => $syscall_function),*), $context_object, $expected_result);
        }
    };
}



#[derive(Debug, Clone)]
pub struct SvmTransactResult<Tx> {
    pub reverted: bool,
    pub program_state: ProgramState,
    pub tx: Tx,
    pub receipts: Vec<Receipt>,
    pub changes: Changes,
}


// [TODO:gmm] From Solana with love

macro_rules! test_interpreter_and_jit {
    (register, $function_registry:expr, $location:expr => $syscall_function:expr) => {
        $function_registry
            .register_function_hashed($location.as_bytes(), $syscall_function)
            .unwrap();
    };
    ($executable:expr, $mem:tt, $context_object:expr, $expected_result:expr $(,)?) => {
        let expected_instruction_count = $context_object.get_remaining();
        #[allow(unused_mut)]
        let mut context_object = $context_object;
        let expected_result = format!("{:?}", $expected_result);
        if !expected_result.contains("ExceededMaxInstructions") {
            context_object.remaining = INSTRUCTION_METER_BUDGET;
        }
        $executable.verify::<RequisiteVerifier>().unwrap();
        let (instruction_count_interpreter, interpreter_final_pc, _tracer_interpreter) = {
            let mut mem = $mem;
            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);
            let mut context_object = context_object.clone();
            create_vm!(
                vm,
                &$executable,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            let (instruction_count_interpreter, result) = vm.execute_program(&$executable, true);
            assert_eq!(
                format!("{:?}", result),
                expected_result,
                "Unexpected result for Interpreter"
            );
            (
                instruction_count_interpreter,
                vm.registers[11],
                vm.context_object_pointer.clone(),
            )
        };
        #[cfg(all(feature = "jit", not(target_os = "windows"), target_arch = "x86_64"))]
        {
            #[allow(unused_mut)]
            let compilation_result = $executable.jit_compile();
            let mut mem = $mem;
            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);
            create_vm!(
                vm,
                &$executable,
                &mut context_object,
                stack,
                heap,
                vec![mem_region],
                None
            );
            match compilation_result {
                Err(err) => assert_eq!(
                    format!("{:?}", err),
                    expected_result,
                    "Unexpected result for JIT compilation"
                ),
                Ok(()) => {
                    let (instruction_count_jit, result) = vm.execute_program(&$executable, false);
                    let tracer_jit = &vm.context_object_pointer;
                    if !TestContextObject::compare_trace_log(&_tracer_interpreter, tracer_jit) {
                        let analysis = Analysis::from_executable(&$executable).unwrap();
                        let stdout = std::io::stdout();
                        analysis
                            .disassemble_trace_log(
                                &mut stdout.lock(),
                                &_tracer_interpreter.trace_log,
                            )
                            .unwrap();
                        analysis
                            .disassemble_trace_log(&mut stdout.lock(), &tracer_jit.trace_log)
                            .unwrap();
                        panic!();
                    }
                    assert_eq!(
                        format!("{:?}", result),
                        expected_result,
                        "Unexpected result for JIT"
                    );
                    assert_eq!(
                        instruction_count_interpreter, instruction_count_jit,
                        "Interpreter and JIT instruction meter diverged",
                    );
                    assert_eq!(
                        interpreter_final_pc, vm.registers[11],
                        "Interpreter and JIT instruction final PC diverged",
                    );
                }
            }
        }
        if $executable.get_config().enable_instruction_meter {
            assert_eq!(
                instruction_count_interpreter, expected_instruction_count,
                "Instruction meter did not consume expected amount"
            );
        }
    };
}

macro_rules! test_interpreter_and_jit_asm {
    ($source:tt, $config:expr, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            let mut config = $config;
            config.enable_instruction_tracing = true;
            let mut function_registry = FunctionRegistry::<BuiltinFunction<TestContextObject>>::default();
            $(test_interpreter_and_jit!(register, function_registry, $location => $syscall_function);)*
            let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
            let mut executable = assemble($source, loader).unwrap();
            test_interpreter_and_jit!(executable, $mem, $context_object, $expected_result);
        }
    };
    ($source:tt, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            test_interpreter_and_jit_asm!($source, Config::default(), $mem, ($($location => $syscall_function),*), $context_object, $expected_result);
        }
    };
}

fn example_asm(source: &str) -> Vec<u8> {
    let loader = Arc::new(BuiltinProgram::new_loader(
        Config {
            enable_symbol_and_section_labels: true,
            ..Config::default()
        },
        FunctionRegistry::default(),
    ));

    let executable = assemble::<TestContextObject>(source, loader).unwrap();

    let (_, bytecode) = executable.get_text_bytes();

    bytecode.to_vec()
}


fn example_disasm_from_bytes(program: &[u8]) {
    let loader = Arc::new(BuiltinProgram::new_mock());
    let executable = Executable::<TestContextObject>::from_text_bytes(
        program,
        loader,
        SBPFVersion::V2,
        FunctionRegistry::default(),
    ).unwrap();
    let analysis = Analysis::from_executable(&executable).unwrap();
    let stdout = std::io::stdout();
    analysis.disassemble(&mut stdout.lock()).unwrap();
}

fn execute_generated_program(prog: &[u8], mem: &mut [u8]) -> Option<Vec<u8>> {
    let max_instruction_count = 1024;
    let executable =
        Executable::<TestContextObject>::from_text_bytes(
            prog,
            Arc::new(BuiltinProgram::new_loader(
                Config {
                    enable_instruction_tracing: true,
                    ..Config::default()
                },
                FunctionRegistry::default(),
            )),
            SBPFVersion::V2,
            FunctionRegistry::default(),
        );

    let mut executable = if let Ok(executable) = executable {
        executable
    } else {
        return None;
    };

    if executable.verify::<RequisiteVerifier>().is_err() || executable.jit_compile().is_err() {
        return None;
    }

    let (instruction_count_interpreter, tracer_interpreter, result_interpreter) = {
        let mut context_object = TestContextObject::new(max_instruction_count);
        let mem_region = MemoryRegion::new_writable(mem, ebpf::MM_INPUT_START);
        create_vm!(
            vm,
            &executable,
            &mut context_object,
            stack,
            heap,
            vec![mem_region],
            None
        );

        let (instruction_count_interpreter, result_interpreter) =
            vm.execute_program(&executable, true);

        let tracer_interpreter = vm.context_object_pointer.clone();
        (
            instruction_count_interpreter,
            tracer_interpreter,
            result_interpreter,
        )
    };

    // JIT

    let mut context_object = TestContextObject::new(max_instruction_count);
    let mem_region = MemoryRegion::new_writable(mem, ebpf::MM_INPUT_START);

    create_vm!(
        vm,
        &executable,
        &mut context_object,
        stack,
        heap,
        vec![mem_region],
        None
    );

    let (instruction_count_jit, result_jit) = vm.execute_program(&executable, false);
    let tracer_jit = &vm.context_object_pointer;

    if format!("{result_interpreter:?}") != format!("{result_jit:?}")
        || !TestContextObject::compare_trace_log(&tracer_interpreter, tracer_jit)
    {
        let analysis =
            solana_rbpf::static_analysis::Analysis::from_executable(&executable).unwrap();
        println!("result_interpreter={result_interpreter:?}");
        println!("result_jit={result_jit:?}");
        let stdout = std::io::stdout();
        analysis
            .disassemble_trace_log(&mut stdout.lock(), &tracer_interpreter.trace_log)
            .unwrap();
        analysis
            .disassemble_trace_log(&mut stdout.lock(), &tracer_jit.trace_log)
            .unwrap();
        panic!();
    }
    if executable.get_config().enable_instruction_meter {
        assert_eq!(instruction_count_interpreter, instruction_count_jit);
    }

    Some(mem.to_vec())
}

fn example_mov() {
    test_interpreter_and_jit_asm!(
        "
        mov32 r1, 1
        mov32 r0, r1
        exit",
        [],
        (),
        TestContextObject::new(3),
        ProgramResult::Ok(0x1),
    );
}

fn example_add32() {
    test_interpreter_and_jit_asm!(
        "
        mov32 r0, 0
        mov32 r1, 2
        add32 r0, 1
        add32 r0, r1
        exit",
        [],
        (),
        TestContextObject::new(5),
        ProgramResult::Ok(0x3),
    );
}

fn test_struct_func_pointer() {
    // This tests checks that a struct field adjacent to another field
    // which is a relocatable function pointer is not overwritten when
    // the function pointer is relocated at load time.
    let config = Config {
        enable_instruction_tracing: true,
        ..Config::default()
    };
    // let mut file = File::open("struct_func_pointer.so").unwrap();
    let mut file = File::open("/home/rigidus/src/hello_world/target/deploy/hello_world.so").unwrap();
    let mut elf = Vec::new();
    file.read_to_end(&mut elf).unwrap();

    println!("ELF file loaded successfully. Size: {}", elf.len());

    #[allow(unused_mut)]
    {
        // Holds the function symbols of an Executable
        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<TestContextObject>>::default();
        // Регистрация системного вызова
        function_registry
            .register_function_hashed(*b"bpf_mem_frob", syscalls::SyscallMemFrob::vm);

        // function_registry
        //     .register_function_hashed(*b"log", log);
        // Constructs a loader built-in program
        let loader =
            Arc::new(BuiltinProgram::new_loader(config, function_registry));
        // Creates an executable from an ELF file
        let mut executable =
            Executable::<TestContextObject>::from_elf(&elf, loader).unwrap();

        println!("Executable created successfully.");

        // Counting instructions
        let expected_instruction_count =
            (TestContextObject::new(3)).get_remaining();
        #[allow(unused_mut)]
        let mut context_object = TestContextObject::new(3);
        // Result
        let expected_result = format!("{:?}", (ProgramResult::Ok(0x102030405060708)));
        if !expected_result.contains("ExceededMaxInstructions") {
            context_object.remaining = INSTRUCTION_METER_BUDGET;
        }
        executable.verify::<RequisiteVerifier>().unwrap();

        println!("Executable verified successfully.");

        let (instruction_count_interpreter, interpreter_final_pc, _tracer_interpreter) = {
            // let mut mem = [];
            // let mut mem = vec![0u8; 8];
            // Создаем входную память и инициализируем её
            let mut mem = vec![0u8; 8];  // Размер памяти для input
            mem[0..8].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);  // Пример данных

            let mem_region = MemoryRegion::new_writable(&mut mem, ebpf::MM_INPUT_START);

            println!("Memory region for input: {:?}", mem_region);

            let mut context_object = context_object.clone();
            create_vm!(
                vm,
                & executable ,
                & mut context_object ,
                stack ,
                heap ,
                vec ! [ mem_region ] ,
                None
            );

            println!("Executing program with expected result: {}", expected_result);
            // println!("Memory region for input: {:?}", mem_region);
            let (instruction_count_interpreter, result) = vm.execute_program(&executable, true);
            println!("Execution result: {:?}", result);

            assert_eq!(
                format!("{:?}", result),
                expected_result,
                "Unexpected result for Interpreter"
            );
            (
                instruction_count_interpreter,
                vm.registers[11],
                vm.context_object_pointer.clone(),
            )
        };
        if executable.get_config().enable_instruction_meter {
            assert_eq!(
                instruction_count_interpreter, expected_instruction_count,
                "Instruction meter did not consume expected amount"
            );
        }
    }
}


fn example_syscal() {
    test_interpreter_and_jit_asm!(
        "
        mov r6, r1
        add r1, 2
        mov r2, 4
        syscall bpf_mem_frob
        ldxdw r0, [r6]
        be64 r0
        exit",
        [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, //
        ],
        (
            "bpf_mem_frob" => syscalls::SyscallMemFrob::vm,
        ),
        TestContextObject::new(7),
        ProgramResult::Ok(0x102292e2f2c0708),
    );
}


pub fn svm_transact<'a, Tx, T>(
    storage: &mut T,
    checked_tx: Checked<Tx>,
    header: &'a PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    memory: &'a mut MemoryInstance,
    consensus_params: ConsensusParameters,
    extra_tx_checks: bool,
    execution_data: &mut ExecutionData,
) -> core::result::Result<SvmTransactResult<Tx>, Error>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: KeyValueInspect<Column = Column>,
{
    let execution_options = ExecutionOptions {
        extra_tx_checks,
        backtrace: false,
    };

    let block_executor =
        BlockExecutor::new(WasmRelayer {}, execution_options.clone(), consensus_params)
            .expect("failed to create block executor");

    let structured_storage =
        StructuredStorage::new(storage);
    let mut structured_storage =
        structured_storage.into_transaction();
    let in_memory_transaction =
        InMemoryTransaction::new(
        Changes::new(),
        ConflictPolicy::Overwrite,
        &mut structured_storage,
    );
    let tx_transaction =
        &mut TxStorageTransaction::new(in_memory_transaction);

    let tx_id = checked_tx.id();

    let mut checked_tx = checked_tx;
    if execution_options.extra_tx_checks {
        checked_tx = block_executor.extra_tx_checks(checked_tx, header, tx_transaction, memory)?;
    }

    // Here we go to solana way...

    // test_interpreter_and_jit_asm!(
    //     "
    //     mov32 r1, 1
    //     mov32 r0, r1
    //     exit",
    //     [],
    //     (),
    //     TestContextObject::new(3),
    //     ProgramResult::Ok(0x1),
    // );

    let bytecode = example_asm("
    entrypoint:
        ldxdw r2, [r1+0x00]
        ldxdw r3, [r1+0x08]
        add   r2, r3
        stxdw [r1+0x10], r3
    l_exit:
        exit");

    println!("\n::Generated bytecode:");
    for (i, byte) in bytecode.iter().enumerate() {
        print!("{:#04x} ", byte);
    }
    println!("\n::Disasm:");

    let program: &'static [u8] = &[
        0x79, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x79, 0x13, 0x08, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x0f, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7b, 0x21, 0x10, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    example_disasm_from_bytes(program);

    let mut svm_memory: Vec<u8> = vec![0; 1024 * 1024]; // 1 MB memory

    // Initialize some values in memory for testing
    svm_memory[0x00..0x08].copy_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // Example value 1
    svm_memory[0x08..0x10].copy_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // Example value 2

    if let Some(updated_memory) = execute_generated_program(program, &mut svm_memory) {
        println!("Program executed successfully.");
        println!("Memory content after execution:");
        for (i, byte) in updated_memory.iter().enumerate().take(32) {
            // Display first bytes for example
            println!("Byte {}: {:#04x}", i, byte);
        }
    } else {
        println!("Program execution failed.");
    }

    example_mov();
    example_add32();
    test_struct_func_pointer();
    example_syscal();

    // ----------

    let (reverted, program_state, tx, receipts) =
        block_executor.attempt_tx_execution_with_vm(
            checked_tx,
            header,
            coinbase_contract_id,
            gas_price,
            tx_transaction,
            memory,
        )?;

    block_executor.spend_input_utxos(tx.inputs(), tx_transaction, reverted, execution_data)?;

    block_executor.persist_output_utxos(
        *header.height(),
        execution_data,
        &tx_id,
        tx_transaction,
        tx.inputs(),
        tx.outputs(),
    )?;

    // tx_st_transaction
    //     .storage::<ProcessedTransactions>()
    //     .insert(&tx_id, &());

    block_executor.update_execution_data(
        &tx,
        execution_data,
        receipts.clone(),
        gas_price,
        reverted,
        program_state,
        tx_id,
    )?;

    Ok(crate::helpers_svm::SvmTransactResult {
        reverted,
        program_state,
        tx,
        receipts,
        changes: tx_transaction.changes().clone(),
    })
}


pub fn svm_transact_commit<Tx, T>(
    storage: &mut T,
    checked_tx: Checked<Tx>,
    header: &PartialBlockHeader,
    coinbase_contract_id: ContractId,
    gas_price: Word,
    consensus_params: ConsensusParameters,
    extra_tx_checks: bool,
    execution_data: &mut ExecutionData,
) -> std::result::Result<SvmTransactResult<Tx>, Error>
where
    Tx: ExecutableTransaction + Cacheable + Send + Sync + 'static,
    <Tx as IntoChecked>::Metadata: CheckedMetadata + Send + Sync,
    T: KeyValueMutate<Column = Column>,
{
    // debug_log!("ecl(svm_transact_commit): start");

    // TODO warmup storage from state based on tx inputs?
    // let inputs = checked_tx.transaction().inputs();
    // for input in inputs {
    //     match input {
    //         Input::CoinSigned(v) => {}
    //         Input::CoinPredicate(v) => {}
    //         Input::Contract(v) => {}
    //         Input::MessageCoinSigned(v) => {}
    //         Input::MessageCoinPredicate(v) => {}
    //         Input::MessageDataSigned(v) => {}
    //         Input::MessageDataPredicate(v) => {}
    //     }
    // }

    let mut memory = MemoryInstance::new();

    let result = svm_transact(
        storage,
        checked_tx,
        header,
        coinbase_contract_id,
        gas_price,
        &mut memory,
        consensus_params,
        extra_tx_checks,
        execution_data,
    )?;

    for (col_num, changes) in &result.changes {
        let column: Column = col_num.clone().try_into().expect("valid column number");
        match column {
            Column::Metadata
            | Column::ContractsRawCode
            | Column::ContractsState
            | Column::ContractsLatestUtxo
            | Column::ContractsAssets
            | Column::ContractsAssetsMerkleData
            | Column::ContractsAssetsMerkleMetadata
            | Column::ContractsStateMerkleData
            | Column::ContractsStateMerkleMetadata
            | Column::Coins => {
                for (key, op) in changes {
                    match op {
                        WriteOperation::Insert(v) => {
                            storage.write(key, column, v)?;
                        }
                        WriteOperation::Remove => {
                            storage.delete(key, column)?;
                        }
                    }
                }
            }

            Column::Transactions
            | Column::FuelBlocks
            | Column::FuelBlockMerkleData
            | Column::FuelBlockMerkleMetadata
            | Column::Messages
            | Column::ProcessedTransactions
            | Column::FuelBlockConsensus
            | Column::ConsensusParametersVersions
            | Column::StateTransitionBytecodeVersions
            | Column::UploadedBytecodes
            | Column::GenesisMetadata => {
                panic!("unsupported column {:?} operation", column)
            }
        }
    }

    Ok(result)
}