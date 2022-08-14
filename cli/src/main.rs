use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::Parser;
use color_eyre::Result;
use colored::Colorize;
mod build;
mod elf;
mod flash;
mod image;
mod logs;
mod xfel;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    match args {
        Args::Build { path } => {
            let mut config = parse_config(&path)?;
            config.build(&path)?;
        }
        Args::Flash { path } => {
            let mut config = parse_config(&path)?;
            let _ = config.build(&path)?;
            flash::flash(&config)?;
            //flash::flash(config.probe, ihex_path)?;
        }
        Args::Logs { path } => {
            let mut config = parse_config(&path)?;
            let target = config.build(&path)?;
            let session = flash::flash(&config)?;
            let kernel_path = target.join("kernel.elf");
            match session {
                flash::Session::Xfel(_xfel) => todo!(),
                flash::Session::Probe(mut session) => {
                    logs::print_logs(&config, kernel_path, &mut session)?;
                }
            }
        }
    }
    Ok(())
}
#[derive(Parser, Debug)]
#[clap(author, version, about = "🏔 - k5's helper tool for flashing, debugging, and building k5 projects", long_about = None)]
enum Args {
    /// Builds a k5 app from the `app.toml` file
    Build {
        /// path to directory containing `app.toml`
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    /// Flashes a k5 app from the `app.toml` file, to the specified chip
    Flash {
        /// path to directory containing `app.toml`
        #[clap(default_value = ".")]
        path: PathBuf,
    },

    /// Flashes and displays logs for a k5 app
    Logs {
        /// path to directory containing `app.toml`
        #[clap(default_value = ".")]
        path: PathBuf,
    },
}

fn parse_config(path: &Path) -> Result<build::Config> {
    let mut config = config::Config::default();
    let path = fs::canonicalize(path)?;
    config.merge(config::File::from(path.join("app.toml")).required(true))?;

    let config: build::Config = config.try_into()?;
    Ok(config)
}

fn print_header(text: impl Into<String>) {
    let text = format!(" {:<20}", text.into());
    // let mut text: String = " ".to_string() + &text.into();
    println!("{}", text.bold().white().on_truecolor(255, 118, 40));
}
