use std::{fs, path::PathBuf};

use clap::Parser;
mod build;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    match args {
        Args::Build { path } => {
            let mut config = config::Config::default();
            let path = fs::canonicalize(path)?;
            config.merge(config::File::from(path.join("app.toml")).required(true))?;

            let mut config: build::Config = config.try_into()?;
            config.build(&path)?;
        }
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
}
