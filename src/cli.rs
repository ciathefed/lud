use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    #[clap(visible_alias = "d", about = "Download a file")]
    Download {
        #[clap(required = true, help = "Remote path to file")]
        input: Utf8PathBuf,

        #[clap(long, short = 'o', help = "Local output of downloaded file")]
        output: Option<Utf8PathBuf>,

        #[clap(long, short = 'f', help = "Overwriting existing local file")]
        force: bool,
    },

    #[clap(visible_alias = "u", about = "Upload a file")]
    Upload {
        #[clap(required = true, help = "Local path to file")]
        input: Utf8PathBuf,

        #[clap(long, short = 'o', help = "Remote output of uploaded file")]
        output: Option<Utf8PathBuf>,

        #[clap(long, short = 'f', help = "Overwriting existing remote file")]
        force: bool,
    },

    #[clap(visible_alias = "ls", about = "List files")]
    List {
        #[clap(help = "Remote path")]
        path: Option<Utf8PathBuf>,
    },

    #[clap(visible_alias = "ln", about = "Start a server")]
    Listen {
        #[clap(
            long,
            short = 'a',
            help = "Listening address",
            default_value = "127.0.0.1:4899"
        )]
        addr: String,

        #[clap(
            long,
            short = 'o',
            help = "Output path for uploaded files",
            default_value = "./storage"
        )]
        output: Utf8PathBuf,
    },

    #[clap(visible_alias = "rm", about = "Delete a file or directory")]
    Remove {
        #[clap(required = true, help = "Remote path to delete")]
        path: Utf8PathBuf,

        #[clap(
            long,
            short = 'f',
            help = "Force deletion (ignore nonexistent files, never prompt)"
        )]
        force: bool,

        #[clap(
            long,
            short = 'r',
            help = "Remove directories and their contents recursively"
        )]
        recursive: bool,
    },

    #[clap(visible_alias = "p", about = "Ping a server")]
    Ping,
}
