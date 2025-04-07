use std::os::unix::fs::MetadataExt;

use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use chrono::Utc;
use humansize::{DECIMAL, format_size};
use tabwriter::TabWriter;
use tokio::{
    fs,
    io::AsyncWriteExt,
    net::{TcpStream, ToSocketAddrs},
    time::Instant,
};

use crate::server::{Connection, File, Packet};

pub async fn download<A: ToSocketAddrs>(
    remote_path: Utf8PathBuf,
    local_path: Option<Utf8PathBuf>,
    force: bool,
    addr: A,
) -> Result<()> {
    let local_path = local_path.unwrap_or_else(|| {
        remote_path.file_name().map(Into::into).unwrap_or_else(|| {
            let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
            format!("{timestamp}-output").into()
        })
    });

    if !force {
        if fs::try_exists(&local_path).await.unwrap_or(false) {
            return Err(anyhow!("File already exists",));
        }
    }

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::Download(remote_path.into(), Vec::new(), 0))
            .await
            .context("Failed to send download request")?;

        match conn.read_packet().await? {
            Packet::Download(_, data, mode) => {
                let mut file = fs::File::create(&local_path)
                    .await
                    .context(format!("Failed to create file `{}`", local_path))?;

                file.write_all(&data)
                    .await
                    .context(format!("Failed to write to file `{}`", local_path))?;

                #[cfg(unix)]
                {
                    use std::fs::Permissions;
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(Permissions::from_mode(mode))
                        .await
                        .context(format!("Failed to set permissions for `{}`", local_path))?;
                }

                log::info!(
                    "Successfully downloaded file `{}` ({})",
                    local_path,
                    format_size(data.len(), DECIMAL)
                );
                Ok(())
            }
            Packet::Error(e) => Err(anyhow!(e)),
            other => Err(anyhow!("Unexpected response: {:?}", other)),
        }
    })
    .await
}

pub async fn upload<A: ToSocketAddrs>(
    local_path: Utf8PathBuf,
    remote_path: Option<Utf8PathBuf>,
    force: bool,
    addr: A,
) -> Result<()> {
    let remote_path = remote_path.unwrap_or_else(|| {
        local_path.file_name().map(Into::into).unwrap_or_else(|| {
            let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
            format!("{timestamp}-output").into()
        })
    });

    let (data, metadata) = tokio::try_join!(fs::read(&local_path), fs::metadata(&local_path),)
        .context(format!("Failed to open file `{}`", &local_path))?;

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::Upload(
            remote_path.into(),
            data,
            metadata.mode(),
            force,
        ))
        .await
        .context("Failed to send packet")?;

        match conn.read_packet().await? {
            Packet::Ok => {
                log::info!(
                    "Successfully uploaded file `{}` ({})",
                    local_path,
                    format_size(metadata.size(), DECIMAL)
                );
                Ok(())
            }
            Packet::Error(e) => Err(anyhow!(e)),
            other => Err(anyhow!("Unexpected response: {:?}", other)),
        }
    })
    .await
}

pub async fn list<A: ToSocketAddrs>(path: Option<Utf8PathBuf>, addr: A) -> Result<()> {
    let path = path.unwrap_or_else(|| "./".into());

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::List(path.clone().into(), Vec::new()))
            .await
            .context("Failed to send list request")?;

        match conn.read_packet().await? {
            Packet::List(_, files) => {
                pretty_print(files);
                Ok(())
            }
            Packet::Error(e) => Err(anyhow!(e)),
            other => Err(anyhow!("Unexpected response: {:?}", other)),
        }
    })
    .await
}

pub async fn ping<A: ToSocketAddrs>(addr: A) -> Result<()> {
    let start_time = Instant::now();

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::Ping)
            .await
            .context("Failed to send ping")?;

        match conn.read_packet().await? {
            Packet::Ok => {
                let duration = start_time.elapsed();
                log::info!("Server is online ({:?})", duration);
                Ok(())
            }
            Packet::Error(e) => Err(anyhow!(e)),
            other => Err(anyhow!("Unexpected response: {:?}", other)),
        }
    })
    .await
}

async fn with_connection<A, F, Fut>(addr: A, operation: F) -> Result<()>
where
    A: ToSocketAddrs,
    F: FnOnce(Connection) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let stream = TcpStream::connect(addr)
        .await
        .context("Failed to connect to server")?;

    operation(Connection::new(stream)).await
}

fn pretty_print(mut files: Vec<File>) {
    use std::io::{self, Write};

    let mut tw = TabWriter::new(io::stdout()).padding(1).minwidth(32);
    let is_tty = atty::is(atty::Stream::Stdout);

    files.sort_by_key(|file| file.size);

    if is_tty {
        writeln!(tw, "\x1b[1mFile Path\tSize\x1b[0m").unwrap();
    } else {
        writeln!(tw, "File Path\tSize").unwrap();
    }

    let mut line = String::with_capacity(128);
    for file in files {
        line.clear();
        if is_tty {
            line.push_str("\x1b[0m");
        }
        line.push_str(&file.path);
        line.push('\t');
        line.push_str(&format_size(file.size, DECIMAL));
        if is_tty {
            line.push_str("\x1b[0m");
        }
        line.push('\n');
        tw.write_all(line.as_bytes()).unwrap();
    }

    tw.flush().unwrap();
}
