use std::{
    fmt::Display,
    fs::Permissions,
    io::ErrorKind,
    net::SocketAddr,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

use anyhow::{Context, Error, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use strum_macros::Display;
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, ToSocketAddrs},
};
use walkdir::WalkDir;

#[derive(Debug, Display, Serialize, Deserialize)]
pub enum Packet {
    Ok,
    Error(String),
    DownloadStart(String, u64, u32),
    DownloadChunk(Vec<u8>),
    DownloadEnd,
    UploadStart(String, u64, u32, bool),
    UploadChunk(Vec<u8>),
    UploadEnd,
    List(String, Vec<File>),
    Remove(String, bool, bool),
    Ping,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub size: u64,
}

pub struct Connection {
    stream: TcpStream,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    pub async fn read_packet(&mut self) -> Result<Packet> {
        let mut len_bytes = [0u8; 4];
        self.stream
            .read_exact(&mut len_bytes)
            .await
            .context("Failed to read length prefix")?;

        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut buffer = vec![0u8; len];
        self.stream
            .read_exact(&mut buffer)
            .await
            .context("Failed to read packet data")?;

        let packet: Packet =
            bincode::deserialize(&buffer).context("Failed to deserialize packet")?;

        Ok(packet)
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        let bytes = bincode::serialize(packet).context("Failed to serialize packet")?;

        let len = bytes.len() as u32;
        self.stream
            .write_all(&len.to_be_bytes())
            .await
            .context("Failed to write length prefix")?;

        self.stream
            .write_all(&bytes)
            .await
            .context("Failed to write packet data")?;

        Ok(())
    }

    pub async fn shutdown(&mut self) {
        let _ = self.stream.shutdown().await;
    }
}

pub async fn start<A: ToSocketAddrs + Display>(addr: A, output_path: Utf8PathBuf) -> Result<()> {
    match fs::create_dir_all(&output_path).await {
        Ok(()) => {}
        Err(ref e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(Error::new(e).context("Failed to create output path")),
    }

    let listener = TcpListener::bind(&addr)
        .await
        .context("Failed to start server")?;

    log::info!("Server started on {}", addr);

    loop {
        let (stream, addr) = listener
            .accept()
            .await
            .context("Failed to accept connection")?;

        log::info!("Accepted connection from {}", addr);

        let output_path = output_path.clone();
        tokio::spawn(async move {
            handle_connection(stream, addr, output_path).await;
        });
    }
}

async fn send_error(conn: &mut Connection, msg: &str) {
    if let Err(e) = conn.write_packet(&Packet::Error(msg.into())).await {
        log::error!("Failed to send packet: {:#}", e);
    }
}

async fn send_ok(conn: &mut Connection) {
    if let Err(e) = conn.write_packet(&Packet::Ok).await {
        log::error!("Failed to send packet: {:#}", e);
    }
}

async fn shutdown_connection(conn: &mut Connection, addr: &SocketAddr) {
    conn.shutdown().await;
    log::info!("Closed connection from {}", addr);
}

async fn handle_connection(stream: TcpStream, addr: SocketAddr, output_path: Utf8PathBuf) {
    let mut conn = Connection::new(stream);

    async {
        let packet = match conn.read_packet().await {
            Ok(p) => p,
            Err(e) => {
                send_error(&mut conn, "Failed to read packet").await;
                log::error!("Failed to read packet: {:#}", e);
                return;
            }
        };
        let packet_name = format!("{}", packet);

        match packet {
            Packet::DownloadStart(file_path, _, _) => {
                if let Err(e) = handle_download(&mut conn, &output_path, &file_path, &addr).await {
                    log::error!("Download failed: {:#}", e);
                } else {
                    send_ok(&mut conn).await;
                }
            }

            Packet::UploadStart(file_path, total_size, mode, force) => {
                if let Err(e) = handle_upload(
                    &mut conn,
                    &output_path,
                    file_path,
                    total_size,
                    mode,
                    force,
                    &addr,
                )
                .await
                {
                    log::error!("Upload failed: {:#}", e);
                } else {
                    send_ok(&mut conn).await;
                }
            }

            Packet::List(path, _) => {
                if let Err(e) = handle_list(&mut conn, &output_path, path).await {
                    log::error!("List failed: {:#}", e);
                } else {
                    send_ok(&mut conn).await;
                }
            }

            Packet::Remove(path, force, recursive) => {
                if let Err(e) = handle_remove(&mut conn, &output_path, path, force, recursive).await
                {
                    log::error!("Remove failed: {:#}", e);
                } else {
                    send_ok(&mut conn).await;
                }
            }

            Packet::Ping => {
                send_ok(&mut conn).await;
            }

            _ => {
                send_error(&mut conn, &format!("Unsupported packet `{}`", packet)).await;
            }
        }

        log::info!(
            "Successfully handled packet `{}` from {}",
            packet_name,
            addr
        );
    }
    .await;

    shutdown_connection(&mut conn, &addr).await;
}

async fn handle_download(
    conn: &mut Connection,
    output_path: &Utf8Path,
    file_path: &str,
    addr: &SocketAddr,
) -> Result<()> {
    let full_path = match safe_join(output_path, file_path) {
        Some(p) => p,
        None => {
            send_error(conn, "Invalid file path").await;
            anyhow::bail!("Invalid file path provided: {}", file_path);
        }
    };

    let metadata = fs::metadata(&full_path)
        .await
        .context("Failed to get file metadata")?;
    let file_size = metadata.len();

    #[cfg(unix)]
    let mode = metadata.mode();
    #[cfg(not(unix))]
    let mode = 0;

    conn.write_packet(&Packet::DownloadStart(
        file_path.to_string(),
        file_size,
        mode,
    ))
    .await
    .context("Failed to send download start packet")?;

    const CHUNK_SIZE: usize = 64 * 1024;
    let mut file = fs::File::open(&full_path)
        .await
        .context("Failed to open file")?;
    let mut buffer = vec![0u8; CHUNK_SIZE];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .await
            .context("Failed to read file chunk")?;
        if bytes_read == 0 {
            break;
        }

        let chunk = buffer[..bytes_read].to_vec();
        conn.write_packet(&Packet::DownloadChunk(chunk))
            .await
            .context("Failed to send file chunk")?;
    }

    conn.write_packet(&Packet::DownloadEnd)
        .await
        .context("Failed to send download end packet")?;

    log::debug!("Sent file `{}` to {} in chunks", full_path, addr);
    Ok(())
}

async fn handle_upload(
    conn: &mut Connection,
    output_path: &Utf8Path,
    file_path: String,
    total_size: u64,
    mode: u32,
    force: bool,
    addr: &SocketAddr,
) -> Result<()> {
    let full_path = match safe_join(output_path, &file_path) {
        Some(p) => p,
        None => {
            send_error(conn, "Invalid file path").await;
            anyhow::bail!("Invalid file path provided: {}", file_path);
        }
    };

    send_ok(conn).await;

    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .await
            .context("Failed to create directories")?;
    }

    if !force && fs::try_exists(&full_path).await.unwrap_or(false) {
        send_error(conn, "File already exists").await;
        anyhow::bail!("File already exists: {}", full_path);
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&full_path)
        .await
        .context("Failed to create file")?;

    file.set_permissions(Permissions::from_mode(mode))
        .await
        .context("Failed to set file permissions")?;

    let mut received_bytes = 0;
    loop {
        let packet = conn
            .read_packet()
            .await
            .context("Failed to read upload packet")?;

        match packet {
            Packet::UploadChunk(data) => {
                received_bytes += data.len() as u64;
                file.write_all(&data)
                    .await
                    .context("Failed to write file chunk")?;
            }
            Packet::UploadEnd => break,
            _ => {
                send_error(conn, "Unexpected packet during upload").await;
                anyhow::bail!("Unexpected packet during upload");
            }
        }
    }

    if received_bytes != total_size {
        send_error(conn, "File size mismatch").await;
        anyhow::bail!(
            "Received file size {} doesn't match expected size {}",
            received_bytes,
            total_size
        );
    }

    log::debug!("Saved file `{}` from {} in chunks", full_path, addr);
    Ok(())
}

async fn handle_remove(
    conn: &mut Connection,
    output_path: &Utf8Path,
    path: String,
    force: bool,
    recursive: bool,
) -> Result<()> {
    let full_path = match safe_join(&output_path, &path) {
        Some(p) => p,
        None => {
            send_error(conn, "Invalid path").await;
            anyhow::bail!("Invalid path provided: {}", path);
        }
    };

    match fs::try_exists(&full_path).await {
        Ok(false) => {
            if force {
                send_ok(conn).await;
                return Ok(());
            } else {
                send_error(conn, "Path does not exist").await;
                anyhow::bail!("Path `{}` does not exist", full_path);
            }
        }
        Err(e) => {
            send_error(conn, "Failed to check path existence").await;
            anyhow::bail!("Failed to check path `{}`: {:#}", full_path, e);
        }
        _ => {}
    }

    let metadata = match fs::metadata(&full_path).await {
        Ok(m) => m,
        Err(e) => {
            send_error(conn, "Failed to get path metadata").await;
            anyhow::bail!("Failed to get metadata for `{}`: {:#}", full_path, e);
        }
    };

    if metadata.is_file() {
        if let Err(e) = fs::remove_file(&full_path).await {
            send_error(conn, "Failed to delete file").await;
            anyhow::bail!("Failed to delete file `{}`: {:#}", full_path, e);
        }
    } else if metadata.is_dir() {
        if recursive {
            if let Err(e) = fs::remove_dir_all(&full_path).await {
                send_error(conn, "Failed to delete directory recursively").await;
                anyhow::bail!(
                    "Failed to delete directory `{}` recursively: {:#}",
                    full_path,
                    e
                );
            }
        } else {
            match fs::remove_dir(&full_path).await {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {
                    send_error(conn, "Directory not empty (use recursive flag)").await;
                    anyhow::bail!("Directory `{}` not empty", full_path);
                }
                Err(e) => {
                    send_error(conn, "Failed to delete directory").await;
                    anyhow::bail!("Failed to delete directory `{}`: {:#}", full_path, e);
                }
            }
        }
    }

    log::debug!(
        "Deleted path `{}` (force: {}, recursive: {})",
        full_path,
        force,
        recursive
    );

    send_ok(conn).await;

    Ok(())
}

async fn handle_list(conn: &mut Connection, output_path: &Utf8Path, path: String) -> Result<()> {
    let full_path = match safe_join(&output_path, &path) {
        Some(p) => p,
        None => {
            send_error(conn, "Invalid path").await;
            anyhow::bail!("Invalid path provided: {}", path);
        }
    };

    let mut files = Vec::new();

    for entry in WalkDir::new(&full_path).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_dir() {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(stripped_path) = entry.path().strip_prefix(&output_path) {
                    if let Some(path_str) = stripped_path.to_str() {
                        files.push(File {
                            path: path_str.to_string(),
                            size: metadata.size(),
                        });
                    }
                }
            }
        }
    }

    if let Err(e) = conn.write_packet(&Packet::List(path, files)).await {
        anyhow::bail!("Failed to send packet: {:#}", e);
    }

    send_ok(conn).await;

    Ok(())
}

fn safe_join(base: &Utf8Path, relative: &str) -> Option<Utf8PathBuf> {
    if relative.is_empty() || relative == "." || relative == "./" {
        return Some(base.to_owned());
    }

    let relative_path = Utf8Path::new(relative);
    let mut normalized = Utf8PathBuf::new();

    for component in relative_path.components() {
        match component {
            camino::Utf8Component::Prefix(_) | camino::Utf8Component::RootDir => {
                return None;
            }
            camino::Utf8Component::CurDir => {}
            camino::Utf8Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            camino::Utf8Component::Normal(part) => {
                if part.is_empty() {
                    return None;
                }
                normalized.push(part);
            }
        }
    }

    let joined = base.join(&normalized);

    if joined.starts_with(base) {
        Some(joined)
    } else {
        None
    }
}
