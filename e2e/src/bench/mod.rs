mod erc20;
mod greeting;
mod multicall;

use crate::utils::run_with_default_context;
use hex_literal::hex;
use sp1_sdk::{ProverClient, SP1Stdin};

#[test]
#[ignore]
fn test_example_keccak_sp1() {
    pub const FIBONACCI_ELF: &[u8] =
        include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");

    sp1_sdk::utils::setup_logger();

    let client = ProverClient::new();

    let input_data = include_bytes!("../../../examples/keccak/lib.wasm");

    let input = "Hello World";

    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let mut stdin = SP1Stdin::new();
    stdin.write(&input_bytes);

    let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();

    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_sp1.to_vec(), output);

    println!(
        "SP1 opcode counts: {:?}",
        report.opcode_counts.as_slice().iter().sum::<u64>()
    );
}

#[test]
#[ignore]
fn test_example_greeting_sp1() {
    pub const FIBONACCI_ELF: &[u8] =
        include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");

    sp1_sdk::utils::setup_logger();

    let client = ProverClient::new();

    let input_data = include_bytes!("../../../examples/greeting/lib.wasm");

    let input = "Hello World";

    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let mut stdin = SP1Stdin::new();
    stdin.write(&input_bytes);

    let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();

    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_sp1.to_vec(), output);

    println!(
        "SP1 opcode counts: {:?}",
        report.opcode_counts.as_slice().iter().sum::<u64>()
    );
}

#[test]
#[ignore]
fn test_example_panic_sp1() {
    pub const FIBONACCI_ELF: &[u8] =
        include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");

    sp1_sdk::utils::setup_logger();

    let client = ProverClient::new();

    let input_data = include_bytes!("../../../examples/panic/lib.wasm");

    let mut input_bytes = vec![1, 0];
    input_bytes.append(&mut input_data.to_vec());

    let mut stdin = SP1Stdin::new();
    stdin.write(&input_bytes);

    let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();

    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, -71);

    assert_eq!(output_sp1.to_vec(), output);

    println!(
        "SP1 opcode counts: {:?}",
        report.opcode_counts.as_slice().iter().sum::<u64>()
    );
}

#[test]
#[ignore]
fn test_example_router_sp1() {
    pub const FIBONACCI_ELF: &[u8] =
        include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");

    sp1_sdk::utils::setup_logger();

    let client = ProverClient::new();

    let input_data = include_bytes!("../../../examples/router-solidity/lib.wasm");

    let input = hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000").to_vec();

    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.clone());
    input_bytes.append(&mut input_data.to_vec());

    let mut stdin = SP1Stdin::new();
    stdin.write(&input_bytes);

    let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();

    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_slice());
    assert_eq!(exit_code, 0);

    assert_eq!(output_sp1.to_vec(), output);

    println!(
        "SP1 opcode counts: {:?}",
        report.opcode_counts.as_slice().iter().sum::<u64>()
    );
}

#[test]
#[ignore]
fn test_example_chess_sp1() {
    pub const FIBONACCI_ELF: &[u8] =
        include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");

    sp1_sdk::utils::setup_logger();

    let client = ProverClient::new();

    let input_data = include_bytes!("../../../examples/shakmaty/lib.wasm");

    let mut input_bytes = vec![1, 0];
    input_bytes.append(&mut input_data.to_vec());

    let mut stdin = SP1Stdin::new();
    stdin.write(&input_bytes);

    let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();

    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, 0);

    assert_eq!(output_sp1.to_vec(), output);

    println!(
        "SP1 opcode counts: {:?}",
        report.opcode_counts.as_slice().iter().sum::<u64>()
    );
}

#[test]
#[ignore]
fn test_example_keccak_rwasm() {
    let input_data = include_bytes!("../../../examples/keccak/lib.wasm");

    let input = "Hello World";

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (_output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_greeting_rwasm() {
    let input_data = include_bytes!("../../../examples/greeting/lib.wasm");

    let input = "Hello World";

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    println!("Compile and Run");
    let (_output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.to_string().into_bytes());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) =
        run_with_default_context(input_data.to_vec(), &input.to_string().into_bytes());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_panic_rwasm() {
    println!("Only compile");
    let input_data = include_bytes!("../../../examples/panic/lib.wasm");
    let mut input_bytes = vec![0, 0];
    input_bytes.append(&mut input_data.to_vec());

    let (_, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let input_data = include_bytes!("../../../examples/panic/lib.wasm");

    let mut input_bytes = vec![1, 0];
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, -71);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, -71);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_router_rwasm() {
    let input_data = include_bytes!("../../../examples/router-solidity/lib.wasm");

    let input = hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000").to_vec();

    println!("Only compile");
    let mut input_bytes = vec![0, input.len() as u8];
    input_bytes.append(&mut input.clone());
    input_bytes.append(&mut input_data.to_vec());

    let (_output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let mut input_bytes = vec![1, input.len() as u8];
    input_bytes.append(&mut input.clone());
    input_bytes.append(&mut input_data.to_vec());

    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_slice());
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}

#[test]
#[ignore]
fn test_example_chess_rwasm() {
    println!("Only compile");
    let input_data = include_bytes!("../../../examples/shakmaty/lib.wasm");
    let is_run = 0;
    let mut input_bytes = vec![is_run, 0];
    input_bytes.append(&mut input_data.to_vec());
    let (_, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Compile and Run");
    let input_data = include_bytes!("../../../examples/shakmaty/lib.wasm");
    let is_run = 1;
    let mut input_bytes = vec![is_run, 0];
    input_bytes.append(&mut input_data.to_vec());
    let (output_jit, exit_code) = run_with_default_context(
        include_bytes!("../../../examples/rwasm-jit/lib.wasm").to_vec(),
        input_bytes.as_slice(),
    );
    assert_eq!(exit_code, 0);

    println!("Run precompile");
    let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
    assert_eq!(exit_code, 0);

    assert_eq!(output_jit, output);
}
