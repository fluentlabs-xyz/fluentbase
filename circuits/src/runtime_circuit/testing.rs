use crate::fluentbase_circuit::FluentbaseCircuit;
use fluentbase_runtime::{Runtime, RuntimeError, ExitCode};
use fluentbase_rwasm::{self as rwasm, rwasm::InstructionSet};
use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

pub(crate) fn test_ok_with_input(mut bytecode: InstructionSet, input: Vec<u8>) {
    bytecode.finalize(true);
    let bytecode: Vec<u8> = bytecode.into();
    let (execution_result, opt_err) = Runtime::run(bytecode.as_slice(), input.as_slice()).unwrap();
    let exit_code_from_data = execution_result.data().exit_code();
    let exit_code = if let Some(RuntimeError::Rwasm(rwasm::Error::Trap(trap))) = &opt_err {
        if let Some(trap_code) = trap.trap_code() {
            let exit_code: ExitCode = trap_code.into();
            exit_code as i32
        } else {
            exit_code_from_data
        }
    } else {
        exit_code_from_data
    };
    println!("OPTERR {:#?}, EXIT_CODE_FROM_DATA {}, EXIT_CODE {}", opt_err, exit_code_from_data, exit_code);
    let circuit = FluentbaseCircuit::from_execution_result_with_exit_code(&execution_result, exit_code);
    let k = 14;
    let instance = vec![Fr::from(exit_code as u64), Fr::from(0), Fr::from(0)];
    let prover = MockProver::<Fr>::run(k, &circuit, vec![instance]).unwrap();
    prover.assert_satisfied();
}

pub(crate) fn test_ok(bytecode: InstructionSet) {
    test_ok_with_input(bytecode, vec![]);
}
