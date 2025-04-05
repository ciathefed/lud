use std::{
    fmt::Display,
    fs::Permissions,
    io::ErrorKind,
    net::SocketAddr,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

use anyhow::{Context, Error, Result};
use camino::Utf8PathBuf;
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
    Download(String, Vec<u8>, u32),
    Upload(String, Vec<u8>, u32, bool),
    List(String, Vec<File>),
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
        let packet = conn.read_packet().await.unwrap();
        let packet_name = format!("{}", packet);

        match packet {
            Packet::Download(file_path, _, _) => {
                let full_path = output_path.join(&file_path);

                match fs::read(&full_path).await {
                    Ok(data) => {
                        let metadata = match fs::metadata(&full_path).await {
                            Ok(m) => m,
                            Err(e) => {
                                send_error(&mut conn, "Failed to get file metadata").await;
                                log::error!("Failed to get metadata for `{}`: {:#}", full_path, e);
                                return;
                            }
                        };

                        #[cfg(unix)]
                        let mode = metadata.mode();
                        #[cfg(not(unix))]
                        let mode = 0;

                        if let Err(e) = conn
                            .write_packet(&Packet::Download(file_path, data, mode))
                            .await
                        {
                            log::error!("Failed to send file `{}`: {:#}", full_path, e);
                            return;
                        }

                        log::info!("Sent file `{}` to {}", full_path, addr);
                        send_ok(&mut conn).await;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        send_error(&mut conn, "File not found").await;
                        log::error!("File `{}` not found for download", full_path);
                    }
                    Err(e) => {
                        send_error(&mut conn, "Failed to read file").await;
                        log::error!("Failed to read file `{}`: {:#}", full_path, e);
                    }
                }
            }

            Packet::Upload(file_path, data, mode, force) => {
                let full_path = output_path.join(&file_path);

                if !force {
                    match fs::try_exists(&full_path).await {
                        Ok(true) => {
                            send_error(&mut conn, "File already exists").await;
                            return;
                        }
                        Err(e) => {
                            send_error(
                                &mut conn,
                                "Failed to confirm whether the file already exists",
                            )
                            .await;
                            log::error!(
                                "Failed to confirm whether file `{}` exists: {:#}",
                                full_path,
                                e
                            );
                            return;
                        }
                        _ => {}
                    }
                }

                let file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&full_path)
                    .await;

                match file {
                    Ok(mut f) => {
                        let _ = f.set_permissions(Permissions::from_mode(mode)).await;
                        if let Err(e) = f.write_all(&data).await {
                            send_error(&mut conn, "Failed to write file").await;
                            log::error!("Failed to write to `{}`: {:#}", full_path, e);
                            return;
                        }
                    }
                    Err(e) => {
                        send_error(&mut conn, "Failed to create file").await;
                        log::error!("Failed to create file `{}`: {:#}", full_path, e);
                        return;
                    }
                }

                log::info!("Saved file `{}`", full_path);
                send_ok(&mut conn).await;
            }

            Packet::List(path, _) => {
                let full_path = output_path.join(&path);

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
                    log::error!("Failed to send packet: {:#}", e);
                }

                send_ok(&mut conn).await;
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
