use anyhow::Result;
use clap::Parser as _;
use cli::{Cli, Command};
use list::select_server_from_list;
use log::LevelFilter;
use settings::Settings;
use simplelog::{ColorChoice, ConfigBuilder, TermLogger, TerminalMode};

mod cli;
mod commands;
mod list;
mod server;
mod settings;
mod utils;

fn init_logger() {
    let log_level = std::env::var("LUD_LOG").unwrap_or_else(|_| String::from("INFO"));

    let level_filter = match log_level.to_uppercase().as_str() {
        "OFF" => LevelFilter::Off,
        "ERROR" => LevelFilter::Error,
        "WARN" => LevelFilter::Warn,
        "INFO" => LevelFilter::Info,
        "DEBUG" => LevelFilter::Debug,
        "TRACE" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    };

    let config = ConfigBuilder::new()
        .set_time_format_str("%Y-%m-%d %H:%M:%S")
        .set_time_to_local(true)
        .set_target_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Off)
        .build();

    TermLogger::init(level_filter, config, TerminalMode::Mixed, ColorChoice::Auto).unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();

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

    log::debug!("Using server `{}`", server.name);

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
        Command::Remove {
            path,
            force,
            recursive,
        } => run_or_exit(commands::remove(path, force, recursive, addr)).await,
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
