use camino::{Utf8Path, Utf8PathBuf};
use humansize::{BINARY, format_size};
use tabwriter::TabWriter;

use crate::server::File;

pub fn pretty_print(mut files: Vec<File>) {
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
        line.push_str(&format_size(file.size, BINARY));
        if is_tty {
            line.push_str("\x1b[0m");
        }
        line.push('\n');
        tw.write_all(line.as_bytes()).unwrap();
    }

    tw.flush().unwrap();
}

pub fn optimal_chunk_size(file_size: u64) -> usize {
    let min = 16 * 1024;
    let max = 1024 * 1024;
    let scaled = ((file_size as f64).log2() * 1024.0).clamp(min as f64, max as f64);
    scaled as usize
}

pub fn safe_join(base: &Utf8Path, relative: &str) -> Option<Utf8PathBuf> {
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
