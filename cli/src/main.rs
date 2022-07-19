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
mod logs;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    match args {
        Args::Build { path } => {
            let mut config = parse_config(&path)?;
            config.build(&path)?;
        }
        Args::Flash { path, probe } => {
            let mut config = parse_config(&path)?;
            let target = config.build(&path)?;
            let ihex_path = target.join("final.ihex");
            config.probe.merge(probe);
            flash::flash(config.probe, ihex_path)?;
        }
        Args::Logs { path, probe } => {
            let mut config = parse_config(&path)?;
            let target = config.build(&path)?;
            let ihex_path = target.join("final.ihex");
            config.probe.merge(probe);
            let mut session = flash::flash(config.probe.clone(), ihex_path)?;
            let kernel_path = target.join("kernel.elf");
            logs::print_logs(&config, kernel_path, &mut session)?;
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
        #[clap(flatten)]
        probe: flash::FlashConfig,
    },

    /// Flashes and displays logs for a k5 app
    Logs {
        /// path to directory containing `app.toml`
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

fn print_header(text: impl Into<String>) {
    let text = format!(" {:<20}", text.into());
    // let mut text: String = " ".to_string() + &text.into();
    println!("{}", text.bold().white().on_truecolor(255, 118, 40));
}
