use anyhow::Result;
use config::{Config, File};
use serde::Deserialize;
use std::{
    env,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub servers: Vec<Server>,
}

#[derive(Debug, Deserialize)]
pub struct Server {
    pub name: String,
    pub addr: String,
    #[serde(default)]
    pub default: bool,
}

fn get_search_paths() -> Result<Vec<PathBuf>> {
    let mut search_paths: Vec<PathBuf> = vec![env::current_dir()?];

    #[cfg(unix)]
    {
        if let Some(home) = env::var_os("HOME") {
            let config_path = Path::new(&home).join(".config");
            search_paths.push(config_path);
        }
    }

    #[cfg(windows)]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            let config_path = Path::new(&appdata)
                .join(env!("CARGO_PKG_NAME"))
                .join("Config");
            search_paths.push(config_path);
        }
    }

    Ok(search_paths)
}

pub fn try_load_config_file() -> Result<Config> {
    let paths = get_search_paths()?;

    let mut builder = Config::builder();

    for path in paths {
        let file = path.join("rup.toml");
        if file.exists() {
            builder = builder.add_source(File::from(file));
            break;
        }
    }

    Ok(builder.build()?)
}
