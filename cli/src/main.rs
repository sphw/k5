use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::Parser;
use color_eyre::Result;
use probe_rs_cli_util::common_options::ProbeOptions;
mod build;
mod flash;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    match args {
        Args::Build { path } => {
            let mut config = parse_config(&path)?;
            config.build(&path)?;
        }
        Args::Flash { path, probe } => {}
    }
    Ok(())
}
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
enum Args {
    Build {
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    Flash {
        #[clap(default_value = ".")]
        path: PathBuf,
        #[clap(flatten)]
        probe: flash::FlashConfig,
    },
}

fn parse_config(path: &Path) -> Result<build::Config> {
    let mut config = config::Config::default();
    let path = fs::canonicalize(path)?;
    config.merge(config::File::from(path.join("app.toml")).required(true))?;

    let config: build::Config = config.try_into()?;
    Ok(config)
}
