#![allow(dead_code)]

#[path = "../inspector.rs"]
mod inspector;
#[path = "../merkle_trie.rs"]
pub mod merkle_trie;
#[path = "../runner.rs"]
mod runner;
#[path = "../state.rs"]
mod state;
#[path = "../utils.rs"]
pub mod utils;

use runner::{execute_evm_test_suite, execute_fluent_test_suite, find_all_json_tests};
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(
    name = "eth-replay",
    about = "Replay Ethereum transaction fixtures against Fluent EVM"
)]
struct Cmd {
    /// Path to a replay fixture JSON file or a directory containing fixture JSON files.
    #[structopt(required = true)]
    path: Vec<PathBuf>,

    /// Run Fluent with the trace inspector.
    #[structopt(long)]
    trace: bool,

    /// Emit per-test JSON outcome records from the underlying runner.
    #[structopt(short = "o", long)]
    json_outcome: bool,

    /// Also run the reference EVM path and compare it against Fluent.
    #[structopt(long)]
    compare_reference: bool,

    /// Stop after the first failed fixture.
    #[structopt(long)]
    fail_fast: bool,

    /// Write one JSON object per replayed fixture.
    #[structopt(long, parse(from_os_str))]
    report_jsonl: Option<PathBuf>,
}

#[derive(Debug)]
struct ReplayResult {
    path: PathBuf,
    ok: bool,
    elapsed: Duration,
    error: Option<String>,
}

impl Cmd {
    fn run(&self) -> io::Result<bool> {
        let files = self.fixture_files();
        let started = Instant::now();
        let mut report = self.open_report()?;
        let mut results = Vec::with_capacity(files.len());
        let elapsed = Arc::new(Mutex::new(Duration::ZERO));

        for path in files {
            let result = self.run_fixture(&path, &elapsed);
            self.write_report(&mut report, &result)?;
            let failed = !result.ok;
            results.push(result);

            if failed && self.fail_fast {
                break;
            }
        }

        self.print_summary(&results, started.elapsed(), *elapsed.lock().unwrap());
        Ok(results.iter().all(|result| result.ok))
    }

    fn fixture_files(&self) -> Vec<PathBuf> {
        let mut files = self
            .path
            .iter()
            .flat_map(|path| find_all_json_tests(path))
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    fn run_fixture(&self, path: &Path, elapsed: &Arc<Mutex<Duration>>) -> ReplayResult {
        let started = Instant::now();
        let outcome = if self.compare_reference {
            execute_evm_test_suite(path, elapsed, self.trace, self.json_outcome)
        } else {
            execute_fluent_test_suite(path, elapsed, self.trace, self.json_outcome)
        };

        match outcome {
            Ok(()) => ReplayResult {
                path: path.to_path_buf(),
                ok: true,
                elapsed: started.elapsed(),
                error: None,
            },
            Err(error) => ReplayResult {
                path: path.to_path_buf(),
                ok: false,
                elapsed: started.elapsed(),
                error: Some(error.to_string()),
            },
        }
    }

    fn open_report(&self) -> io::Result<Option<File>> {
        self.report_jsonl
            .as_ref()
            .map(|path| {
                OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(path)
            })
            .transpose()
    }

    fn write_report(&self, report: &mut Option<File>, result: &ReplayResult) -> io::Result<()> {
        let Some(report) = report else {
            return Ok(());
        };

        let record = json!({
            "path": result.path,
            "engine": if self.compare_reference { "compare" } else { "fluent" },
            "ok": result.ok,
            "elapsedMillis": result.elapsed.as_millis(),
            "error": result.error,
        });
        writeln!(report, "{record}")
    }

    fn print_summary(&self, results: &[ReplayResult], wall_time: Duration, cpu_time: Duration) {
        let passed = results.iter().filter(|result| result.ok).count();
        let failed = results.len() - passed;
        let engine = if self.compare_reference {
            "compare"
        } else {
            "fluent"
        };

        println!(
            "Replay summary: {} files, {passed} passed, {failed} failed, engine={engine}",
            results.len()
        );
        println!("Wall time: {:.6}s", wall_time.as_secs_f64());
        println!("Total CPU time: {:.6}s", cpu_time.as_secs_f64());

        for result in results.iter().filter(|result| !result.ok) {
            println!(
                "FAILED {}: {}",
                result.path.display(),
                result.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
}

fn main() -> ExitCode {
    let cmd = Cmd::from_args();
    match cmd.run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("eth-replay failed: {error}");
            ExitCode::from(1)
        }
    }
}
