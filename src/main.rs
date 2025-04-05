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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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

    log::info!("Using server `{}`", server.name);

    match cli.cmd {
        Command::Download {
            input,
            output,
            force,
        } => {
            return run_or_exit(commands::download(
                input,
                output,
                force,
                server.addr.clone(),
            ))
            .await;
        }
        Command::Upload {
            input,
            output,
            force,
        } => {
            return run_or_exit(commands::upload(input, output, force, server.addr.clone())).await;
        }
        Command::List { path } => {
            return run_or_exit(commands::list(path, server.addr.clone())).await;
        }
        Command::Ping => {
            return run_or_exit(commands::ping(server.addr.clone())).await;
        }
        _ => {}
    }

    Ok(())
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
