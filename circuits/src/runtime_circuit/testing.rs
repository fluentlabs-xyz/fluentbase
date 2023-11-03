use crate::fluentbase_circuit::FluentbaseCircuit;
use fluentbase_runtime::Runtime;
use fluentbase_rwasm::{self as rwasm, rwasm::InstructionSet};
use halo2_proofs::{dev::MockProver, halo2curves::bn256::Fr};

pub(crate) fn test_ok_with_input(mut bytecode: InstructionSet, input: Vec<u8>) {
    bytecode.finalize(true);
    let bytecode: Vec<u8> = bytecode.into();
    let execution_result = Runtime::run(bytecode.as_slice(), &vec![input]).unwrap();
    let exit_code = execution_result.data().exit_code();
    let circuit =
        FluentbaseCircuit::from_execution_result_with_exit_code(&execution_result, exit_code);
    let k = 14;
    let instance = vec![Fr::from(exit_code as u64), Fr::from(0), Fr::from(0)];
    let prover = MockProver::<Fr>::run(k, &circuit, vec![instance]).unwrap();
    prover.assert_satisfied();
}

pub(crate) fn test_ok(bytecode: InstructionSet) {
    test_ok_with_input(bytecode, vec![]);
}
