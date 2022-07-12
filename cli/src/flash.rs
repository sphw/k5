use std::path::PathBuf;

use probe_rs::{DebugProbeSelector, WireProtocol};
use probe_rs_cli_util::common_options::ProbeOptions;
use serde::{Deserialize, Serialize};

// copy and pasted from probe-rs-cli-util, so we can derive serde
#[derive(Deserialize, clap::Parser, Debug, Default)]
pub struct FlashConfig {
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
    pub dry_run: bool,
    #[structopt(
        long = "allow-erase-all",
        help = "Use this flag to allow all memory, including security keys and 3rd party firmware, to be erased \
        even when it has read-only protection."
    )]
    #[serde(default)]
    pub allow_erase_all: bool,
}

impl FlashConfig {
    pub(crate) fn merge(&mut self, other: FlashConfig) {
        if let Some(chip) = other.chip {
            self.chip = Some(chip)
        }
        if let Some(protocol) = other.protocol {
            self.protocol = Some(protocol)
        }
        if let Some(probe_selector) = other.probe_selector {
            self.probe_selector = Some(probe_selector)
        }
        if let Some(speed) = other.speed {
            self.speed = Some(speed)
        }
        if other.connect_under_reset {
            self.connect_under_reset = other.connect_under_reset
        }
        if other.dry_run {
            self.dry_run = other.dry_run
        }
        if other.allow_erase_all {
            self.allow_erase_all = other.allow_erase_all
        }
    }
}

impl Into<ProbeOptions> for FlashConfig {
    fn into(self) -> ProbeOptions {
        ProbeOptions {
            chip: self.chip,
            chip_description_path: self.chip_description_path,
            protocol: self.protocol,
            probe_selector: self.probe_selector,
            speed: self.speed,
            connect_under_reset: self.connect_under_reset,
            dry_run: self.dry_run,
            allow_erase_all: self.allow_erase_all,
        }
    }
}
