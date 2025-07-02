pub use solana_rbpf::vm::ContextObject;

#[macro_export]
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
            context_object.remaining = crate::helpers::INSTRUCTION_METER_BUDGET;
        }
        $executable
            .verify::<solana_rbpf::verifier::RequisiteVerifier>()
            .unwrap();
        let (instruction_count_interpreter, interpreter_final_pc, _tracer_interpreter) = {
            let mut mem = $mem;
            let mem_region = solana_rbpf::memory_region::MemoryRegion::new_writable(
                &mut mem,
                solana_rbpf::ebpf::MM_INPUT_START,
            );
            let mut context_object = context_object.clone();
            crate::create_vm!(
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
            let mem_region = solana_rbpf::memory_region::MemoryRegion::new_writable(
                &mut mem,
                ebpf::MM_INPUT_START,
            );
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
                    if !solana_rbpf::vm::TestContextObject::compare_trace_log(
                        &_tracer_interpreter,
                        tracer_jit,
                    ) {
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

#[macro_export]
macro_rules! test_interpreter_and_jit_asm {
    ($source:tt, $config:expr, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            let mut config = $config;
            config.enable_instruction_tracing = true;
            let mut function_registry = FunctionRegistry::<solana_rbpf::program::BuiltinFunction<solana_rbpf::vm::TestContextObject>>::default();
            $(crate::test_interpreter_and_jit!(register, function_registry, $location => $syscall_function);)*
            let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
            let mut executable = solana_rbpf::assembler::assemble($source, loader).unwrap();
            crate::test_interpreter_and_jit!(executable, $mem, $context_object, $expected_result);
        }
    };
    ($source:tt, $mem:tt, ($($location:expr => $syscall_function:expr),* $(,)?), $context_object:expr, $expected_result:expr $(,)?) => {
        #[allow(unused_mut)]
        {
            crate::test_interpreter_and_jit_asm!($source, Config::default(), $mem, ($($location => $syscall_function),*), $context_object, $expected_result);
        }
    };
}
