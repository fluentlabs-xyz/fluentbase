mod erc20;
mod greeting;
mod multicall;

// #[test]
// #[ignore]
// fn test_example_keccak_sp1() {
//     pub const FIBONACCI_ELF: &[u8] =
//         include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");
//
//     sp1_sdk::utils::setup_logger();
//
//     let client = ProverClient::new();
//
//     let input_data = include_bytes!("../../../examples/keccak256/lib.wasm");
//
//     let input = "Hello World";
//
//     let mut input_bytes = vec![1, input.len() as u8];
//     input_bytes.append(&mut input.to_string().into_bytes());
//     input_bytes.append(&mut input_data.to_vec());
//
//     let mut stdin = SP1Stdin::new();
//     stdin.write(&input_bytes);
//
//     let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();
//
//     let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
//     assert_eq!(exit_code, 0);
//
//     assert_eq!(output_sp1.to_vec(), output);
//
//     println!(
//         "SP1 opcode counts: {:?}",
//         report.opcode_counts.as_slice().iter().sum::<u64>()
//     );
// }
//
// #[test]
// #[ignore]
// fn test_example_greeting_sp1() {
//     pub const FIBONACCI_ELF: &[u8] =
//         include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");
//
//     sp1_sdk::utils::setup_logger();
//
//     let client = ProverClient::new();
//
//     let input_data = include_bytes!("../../../examples/greeting/lib.wasm");
//
//     let input = "Hello World";
//
//     let mut input_bytes = vec![1, input.len() as u8];
//     input_bytes.append(&mut input.to_string().into_bytes());
//     input_bytes.append(&mut input_data.to_vec());
//
//     let mut stdin = SP1Stdin::new();
//     stdin.write(&input_bytes);
//
//     let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();
//
//     let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_bytes());
//     assert_eq!(exit_code, 0);
//
//     assert_eq!(output_sp1.to_vec(), output);
//
//     println!(
//         "SP1 opcode counts: {:?}",
//         report.opcode_counts.as_slice().iter().sum::<u64>()
//     );
// }
//
// #[test]
// #[ignore]
// fn test_example_panic_sp1() {
//     pub const FIBONACCI_ELF: &[u8] =
//         include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");
//
//     sp1_sdk::utils::setup_logger();
//
//     let client = ProverClient::new();
//
//     let input_data = include_bytes!("../../../examples/panic/lib.wasm");
//
//     let mut input_bytes = vec![1, 0];
//     input_bytes.append(&mut input_data.to_vec());
//
//     let mut stdin = SP1Stdin::new();
//     stdin.write(&input_bytes);
//
//     let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();
//
//     let (output, exit_code) = run_with_default_context(input_data.to_vec(), &[]);
//     assert_eq!(exit_code, -1);
//
//     assert_eq!(output_sp1.to_vec(), output);
//
//     println!(
//         "SP1 opcode counts: {:?}",
//         report.opcode_counts.as_slice().iter().sum::<u64>()
//     );
// }
//
// #[test]
// #[ignore]
// fn test_example_router_sp1() {
//     pub const FIBONACCI_ELF: &[u8] =
//         include_bytes!("../../../examples/sp1/elf/riscv32im-succinct-zkvm-elf");
//
//     sp1_sdk::utils::setup_logger();
//
//     let client = ProverClient::new();
//
//     let input_data = include_bytes!("../../../examples/router-solidity/lib.wasm");
//
//     let input =
// hex!("f8194e480000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e2248656c6c6f2c20576f726c6422000000000000000000000000000000000000"
// ).to_vec();
//
//     let mut input_bytes = vec![1, input.len() as u8];
//     input_bytes.append(&mut input.clone());
//     input_bytes.append(&mut input_data.to_vec());
//
//     let mut stdin = SP1Stdin::new();
//     stdin.write(&input_bytes);
//
//     let (output_sp1, report) = client.execute(FIBONACCI_ELF, stdin).run().unwrap();
//
//     let (output, exit_code) = run_with_default_context(input_data.to_vec(), input.as_slice());
//     assert_eq!(exit_code, 0);
//
//     assert_eq!(output_sp1.to_vec(), output);
//
//     println!(
//         "SP1 opcode counts: {:?}",
//         report.opcode_counts.as_slice().iter().sum::<u64>()
//     );
// }
