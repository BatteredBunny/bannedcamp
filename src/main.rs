use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use bannedcamp::cli::{
    commands::{Cli, Commands},
    completions::generate_completions,
    run::run_download,
};
use bannedcamp::tui;

fn setup_logging(verbosity: u8, quiet: bool) {
    let filter = if quiet {
        EnvFilter::new("error")
    } else {
        match verbosity {
            0 => EnvFilter::new("warn"),
            1 => EnvFilter::new("info"),
            2 => EnvFilter::new("debug"),
            _ => EnvFilter::new("trace"),
        }
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false))
        .with(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    setup_logging(cli.verbose, cli.quiet);

    match cli.command {
        Commands::Library { output } => {
            tui::run(output)?;
        }

        Commands::Download {
            cookie,
            target,
            format,
            output,
            parallel,
            dry_run,
            skip_existing,
        } => {
            run_download(
                cookie,
                target,
                format,
                output,
                parallel,
                dry_run,
                skip_existing,
            )
            .await?;
        }

        Commands::Completions { shell } => {
            generate_completions(shell);
        }
    }

    Ok(())
}
