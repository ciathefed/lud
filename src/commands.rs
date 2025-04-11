use std::os::unix::fs::MetadataExt;

use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use chrono::Utc;
use humansize::{BINARY, format_size};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, ToSocketAddrs},
    time::Instant,
};

use crate::{server::{Connection, Packet}, utils};

const TIME_FORMAT: &str = "%Y-%m-%dT%H-%M-%S";

const PROGRESS_STYLE: &str = "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})";
const PROGRESS_CHARS: &str = "#>-";

pub async fn download<A: ToSocketAddrs>(
    remote_path: Utf8PathBuf,
    local_path: Option<Utf8PathBuf>,
    force: bool,
    addr: A,
) -> Result<()> {
    let local_path = local_path.unwrap_or_else(|| {
        remote_path.file_name().map(Into::into).unwrap_or_else(|| {
            let timestamp = Utc::now().format(TIME_FORMAT).to_string();
            format!("{timestamp}-output").into()
        })
    });

    if !force && fs::try_exists(&local_path).await.unwrap_or(false) {
        return Err(anyhow!("File already exists"));
    }

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::DownloadStart(remote_path.into(), 0, 0))
            .await
            .context("Failed to send download request")?;

        let (_, total_size, mode) = match conn.read_packet().await? {
            Packet::DownloadStart(name, size, mode) => (name, size, mode),
            Packet::Error(e) => return Err(anyhow!(e)),
            other => return Err(anyhow!("Unexpected response: {:?}", other)),
        };

        let mut file = fs::File::create(&local_path)
            .await
            .context(format!("Failed to create file `{}`", local_path))?;

        #[cfg(unix)]
        {
            use std::fs::Permissions;
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(Permissions::from_mode(mode))
                .await
                .context(format!("Failed to set permissions for `{}`", local_path))?;
        }

        let pb = ProgressBar::new(total_size);
        pb.set_style(ProgressStyle::with_template(PROGRESS_STYLE)
            .unwrap()
            .progress_chars(PROGRESS_CHARS));

        let mut received_bytes = 0;

        loop {
            match conn.read_packet().await? {
                Packet::DownloadChunk(data) => {
                    received_bytes += data.len() as u64;
                    file.write_all(&data)
                        .await
                        .context(format!("Failed to write to file `{}`", local_path))?;
                    pb.set_position(received_bytes);
                }
                Packet::DownloadEnd => break,
                Packet::Error(e) => return Err(anyhow!(e)),
                other => return Err(anyhow!("Unexpected packet: {:?}", other)),
            }
        }

        pb.finish_and_clear();

        if received_bytes != total_size {
            return Err(anyhow!(
                "File size mismatch (received {} of {} bytes)",
                received_bytes,
                total_size
            ));
        }

        log::info!(
            "Successfully downloaded file `{}` ({})",
            local_path,
            format_size(total_size, BINARY)
        );
        Ok(())
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
            let timestamp = Utc::now().format(TIME_FORMAT).to_string();
            format!("{timestamp}-output").into()
        })
    });

    let metadata = fs::metadata(&local_path)
        .await
        .context(format!("Failed to get metadata for `{}`", &local_path))?;

    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::UploadStart(
            remote_path.into(),
            metadata.len(),
            metadata.mode(),
            force,
        ))
        .await
        .context("Failed to send upload start packet")?;

        match conn.read_packet().await? {
            Packet::Ok => {}
            Packet::Error(e) => return Err(anyhow!(e)),
            other => return Err(anyhow!("Unexpected response: {:?}", other)),
        }

        let chunk_size = utils::optimal_chunk_size(metadata.len());
        let mut file = fs::File::open(&local_path)
            .await
            .context(format!("Failed to open file `{}`", &local_path))?;

        let pb = ProgressBar::new(metadata.len());
        pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("#>-"));

        let mut buffer = vec![0u8; chunk_size];
        let mut sent_bytes = 0;

        loop {
            let bytes_read = file
                .read(&mut buffer)
                .await
                .context(format!("Failed to read file `{}`", &local_path))?;

            if bytes_read == 0 {
                break;
            }

            conn.write_packet(&Packet::UploadChunk(buffer[..bytes_read].to_vec()))
                .await
                .context("Failed to send file chunk")?;
            
            sent_bytes += bytes_read as u64;
            pb.set_position(sent_bytes);
        }

        pb.finish_and_clear();

        conn.write_packet(&Packet::UploadEnd)
            .await
            .context("Failed to send upload end packet")?;

        match conn.read_packet().await? {
            Packet::Ok => {
                log::info!(
                    "Successfully uploaded file `{}` ({})",
                    local_path,
                    format_size(metadata.len(), BINARY)
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
                utils::pretty_print(files);
                Ok(())
            }
            Packet::Error(e) => Err(anyhow!(e)),
            other => Err(anyhow!("Unexpected response: {:?}", other)),
        }
    })
    .await
}

pub async fn remove<A: ToSocketAddrs>(
    path: Utf8PathBuf,
    force: bool,
    recursive: bool,
    addr: A,
) -> Result<()> {
    with_connection(addr, |mut conn| async move {
        conn.write_packet(&Packet::Remove(path.clone().into(), force, recursive))
            .await
            .context("Failed to send remove request")?;

        match conn.read_packet().await? {
            Packet::Ok => {
                log::info!("Successfully removed path: {}", path);
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
