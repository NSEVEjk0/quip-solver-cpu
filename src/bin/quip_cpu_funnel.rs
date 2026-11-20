//! CPU adaptation of Funnel's staged instance selection and Otsuka anneal.

use clap::Parser;
use quip_solver_core::{run, CommonArgs};
use quip_solver_cpu::families::funnel::{FunnelSampler, CPU_FUNNEL_IDENTITY};
use std::process::ExitCode;

#[derive(Parser)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " cpu-funnel protocol 2"))]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    run(
        CPU_FUNNEL_IDENTITY,
        &cli.common,
        || Ok(FunnelSampler::new()),
    )
}
