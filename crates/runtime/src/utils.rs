// use crate::{ExecutionResult, Runtime};
// use fluentbase_types::ExitCode;
// pub(crate) fn trace_execution_logs(runtime: &Runtime, execution_result: &ExecutionResult) {
//     let trace = runtime.executor.tracer().unwrap().logs.len();
//     println!("execution trace ({} steps):", trace);
//
//     println!("EXEC, interrupted: {}", execution_result.interrupted);
//     println!(
//         "exit_code: {} ({})",
//         execution_result.exit_code,
//         ExitCode::from(execution_result.exit_code)
//     );
//     println!(
//         "output: 0x{} ({})",
//         fluentbase_types::hex::encode(&execution_result.output),
//         std::str::from_utf8(&execution_result.output).unwrap_or("can't decode utf-8")
//     );
//     println!("fuel consumed: {}", execution_result.fuel_consumed);
//     let logs = &runtime.executor.tracer().unwrap().logs;
//     println!("execution trace ({} steps):", logs.len());
//     for log in logs.iter().rev().take(100).rev() {
//         println!(
//             " - pc={} opcode={:?} gas={} stack={:?}",
//             log.program_counter,
//             log.opcode,
//             log.consumed_fuel,
//             log.stack
//                 .iter()
//                 .map(|v| v.to_string())
//                 .rev()
//                 .take(3)
//                 .rev()
//                 .collect::<Vec<_>>(),
//         );
//     }
// }
