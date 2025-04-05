use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use list::select_server_from_list;
use settings::Settings;

mod cli;
mod commands;
mod list;
mod server;
mod settings;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new().parse_env("RUP_LOG").init();

    let cli = Cli::parse();

    if let Command::Listen { addr, output } = cli.cmd {
        return run_or_exit(server::start(addr, output)).await;
    }

    let settings: Settings = settings::try_load_config_file()?.try_deserialize()?;

    let server = settings
        .servers
        .iter()
        .find(|x| x.default)
        .or_else(|| {
            if settings.servers.len() == 1 {
                Some(&settings.servers[0])
            } else {
                select_server_from_list(&settings.servers).ok()
            }
        })
        .unwrap_or_else(|| {
            log::error!("No default server found, and no server was selected.");
            std::process::exit(1);
        });

    let addr = server.addr.clone();

    log::info!("Using server `{}`", server.name);

    match cli.cmd {
        Command::Download {
            input,
            output,
            force,
        } => run_or_exit(commands::download(input, output, force, addr)).await,
        Command::Upload {
            input,
            output,
            force,
        } => run_or_exit(commands::upload(input, output, force, addr)).await,
        Command::List { path } => run_or_exit(commands::list(path, addr)).await,
        Command::Ping => run_or_exit(commands::ping(addr)).await,
        _ => Ok(()),
    }
}

async fn run_or_exit<F>(fut: F) -> Result<()>
where
    F: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    fut.await.map_err(|e| {
        log::error!("{:#}", e);
        std::process::exit(1);
    })
}
