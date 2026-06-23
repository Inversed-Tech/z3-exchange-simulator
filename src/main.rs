use clap::Parser;

use z3_exchange_simulator::cli::{dispatch, init_tracing, Cli, CliError};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    match dispatch(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(CliError::Interrupted) => {
            // Teardown completed cleanly; exit 130 (128 + SIGINT).
            // ExitCode::from(130) runs Rust destructors; std::process::exit would not.
            std::process::ExitCode::from(130u8)
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
