use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use color_eyre::Result;
use colored::Colorize;
use kdam::{tqdm, Column, RichProgress};
use probe_rs::{
    flashing::{DownloadOptions, FlashProgress, ProgressEvent},
    DebugProbeSelector, Session as ProbeSession, WireProtocol,
};
use probe_rs_cli_util::common_options::ProbeOptions;
use serde::Deserialize;

use crate::{
    build::Config,
    xfel::{self, XfelDevice},
};

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "flash_tool")]
#[serde(rename_all = "lowercase")]
pub enum FlashConfig {
    Xfel {
        flash: xfel::InternalFlash,
        base_addr: usize,
    },
    Probe(#[serde(default)] ProbeConfig),
}

// copy and pasted from probe-rs-cli-util, so we can derive serde
#[derive(Deserialize, clap::Parser, Debug, Default, Clone)]
pub struct ProbeConfig {
    #[structopt(long)]
    #[serde(default)]
    pub chip: Option<String>,
    #[structopt(name = "chip description file path", long = "chip-description-path")]
    #[serde(default)]
    pub chip_description_path: Option<PathBuf>,

    /// Protocol used to connect to chip. Possible options: [swd, jtag]
    #[structopt(long, help_heading = "PROBE CONFIGURATION")]
    #[serde(default)]
    pub protocol: Option<WireProtocol>,

    /// Use this flag to select a specific probe in the list.
    ///
    /// Use '--probe VID:PID' or '--probe VID:PID:Serial' if you have more than one probe with the same VID:PID.",
    #[structopt(long = "probe", help_heading = "PROBE CONFIGURATION")]
    #[serde(default)]
    pub probe_selector: Option<DebugProbeSelector>,
    #[clap(
        long,
        help = "The protocol speed in kHz.",
        help_heading = "PROBE CONFIGURATION"
    )]
    pub speed: Option<u32>,
    #[structopt(
        long = "connect-under-reset",
        help = "Use this flag to assert the nreset & ntrst pins during attaching the probe to the chip."
    )]
    #[serde(default)]
    pub connect_under_reset: bool,
    #[structopt(long = "dry-run")]
    #[serde(default)]
    pub dry_run: bool,
    #[structopt(
        long = "allow-erase-all",
        help = "Use this flag to allow all memory, including security keys and 3rd party firmware, to be erased \
        even when it has read-only protection."
    )]
    #[serde(default)]
    pub allow_erase_all: bool,
}

// impl FlashConfig {
//     pub(crate) fn merge(&mut self, other: FlashConfig) {
//         match (self, other) {
//             (_, other) => {
//                 *self = other;
//             }
//             (FlashConfig::Probe(l), FlashConfig::Probe(r)) => l.merge(r),
//         }
//     }
// }

// impl ProbeConfig {
// pub(crate) fn merge(&mut self, other: ProbeConfig) {
//     if let Some(chip) = other.chip {
//         self.chip = Some(chip)
//     }
//     if let Some(protocol) = other.protocol {
//         self.protocol = Some(protocol)
//     }
//     if let Some(probe_selector) = other.probe_selector {
//         self.probe_selector = Some(probe_selector)
//     }
//     if let Some(speed) = other.speed {
//         self.speed = Some(speed)
//     }
//     if other.connect_under_reset {
//         self.connect_under_reset = other.connect_under_reset
//     }
//     if other.dry_run {
//         self.dry_run = other.dry_run
//     }
//     if other.allow_erase_all {
//         self.allow_erase_all = other.allow_erase_all
//     }
// }
// }

impl From<ProbeConfig> for ProbeOptions {
    fn from(val: ProbeConfig) -> Self {
        ProbeOptions {
            chip: val.chip,
            chip_description_path: val.chip_description_path,
            protocol: val.protocol,
            probe_selector: val.probe_selector,
            speed: val.speed,
            connect_under_reset: val.connect_under_reset,
            dry_run: val.dry_run,
            allow_erase_all: val.allow_erase_all,
        }
    }
}

pub fn flash(config: &Config) -> Result<Session> {
    let target = config.kernel.crate_path.join("target");
    match &config.flash_probe {
        FlashConfig::Xfel { flash, base_addr } => {
            let device = XfelDevice::connect(*flash).map_err(|err| {
                println!("{} {}", " Hint ".on_red().white(), "Remember to put the device into FEL mode, this can be done by holding down the FEL button and hitting reset.");
               err
            })?;
            let bin = target.join("final.bin");
            device.write_flash(*base_addr, &bin)?;
            Ok(Session::Xfel(device))
        }
        FlashConfig::Probe(probe) => {
            let probe = probe.clone();
            let ihex = target.join("final.ihex");
            probe.flash(ihex).map(Session::Probe)
        }
    }
}

pub enum Session {
    Xfel(XfelDevice),
    Probe(ProbeSession),
}

impl ProbeConfig {
    pub fn flash(self, ihex: PathBuf) -> Result<ProbeSession> {
        crate::print_header("Flashing");
        let config: ProbeOptions = self.into();
        let mut session = config.simple_attach()?;
        let mut bin = fs::File::open(ihex)?;
        let mut loader = session.target().flash_loader();
        loader.load_hex_data(&mut bin)?;
        let mut download_option = DownloadOptions::default();
        //download_option.keep_unwritten_bytes = config.restore_unwritten;
        download_option.dry_run = config.dry_run;
        download_option.do_chip_erase = true;
        // download_option.disable_double_buffering = config.disable_double_buffering;

        let pb = Arc::new(Mutex::new(RichProgress::new(
            tqdm!(),
            vec![
                Column::Bar,
                Column::Percentage(1),
                Column::Text("•".to_string(), None),
                Column::CountTotal,
                Column::Text("•".to_string(), None),
                Column::RemainingTime,
            ],
        )));
        kdam::term::init();
        let total_sector_size = Arc::new(Mutex::new(0));
        let total_page_size = Arc::new(Mutex::new(0));
        let total_fill_size = Arc::new(Mutex::new(0));
        let progress = FlashProgress::new(move |event| match event {
            ProgressEvent::Initialized { flash_layout } => {
                *total_page_size.lock().unwrap() =
                    flash_layout.pages().iter().map(|s| s.size()).sum();
                *total_sector_size.lock().unwrap() =
                    flash_layout.sectors().iter().map(|s| s.size()).sum();
                *total_fill_size.lock().unwrap() =
                    flash_layout.fills().iter().map(|s| s.size()).sum();
            }
            ProgressEvent::StartedFilling => {
                let mut pb = pb.lock().unwrap();
                pb.reset(Some(*total_fill_size.lock().unwrap() as usize))
            }
            ProgressEvent::StartedErasing => {
                let mut pb = pb.lock().unwrap();
                pb.reset(Some(*total_sector_size.lock().unwrap() as usize))
            }
            ProgressEvent::StartedProgramming => {
                let mut pb = pb.lock().unwrap();
                pb.reset(Some(*total_page_size.lock().unwrap() as usize))
            }

            ProgressEvent::PageFilled { size, .. } => {
                let mut pb = pb.lock().unwrap();
                pb.update(size as usize);
            }
            ProgressEvent::SectorErased { size, .. } => {
                let mut pb = pb.lock().unwrap();
                pb.update(size as usize);
            }
            ProgressEvent::PageProgrammed { size, .. } => {
                let mut pb = pb.lock().unwrap();
                pb.update(size as usize);
            }
            _ => {}
        });
        download_option.progress = Some(&progress);
        loader.commit(&mut session, download_option)?;

        Ok(session)
    }
}
