use anyhow::Result;
use clap::Parser as _;
use cli::{Cli, Command};

mod cli;
mod commands;
mod server;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // env_logger::init();

    let cli = Cli::parse();

    match cli.cmd {
        Command::Download {
            input,
            output,
            force,
        } => {
            run_or_exit(commands::download(input, output, force, "0.0.0.0:4899")).await;
        }
        Command::Upload {
            input,
            output,
            force,
        } => {
            run_or_exit(commands::upload(input, output, force, "0.0.0.0:4899")).await;
        }
        Command::Listen { addr, output } => {
            run_or_exit(server::start(addr, output)).await;
        }
        Command::List { path } => {
            run_or_exit(commands::list(path, "0.0.0.0:4899")).await;
        }
        Command::Ping => {
            run_or_exit(commands::ping("0.0.0.0:4899")).await;
        }
    }

    Ok(())
}

async fn run_or_exit<F>(fut: F)
where
    F: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    if let Err(e) = fut.await {
        log::error!("{:#}", e);
        std::process::exit(1);
    }
}
