use clap::Parser;
use fluentbase_build::{execute_build, BuildArgs};

fn main() -> anyhow::Result<()> {
    let args = BuildArgs::parse();
    execute_build(&args, None)?;
    Ok(())
}
