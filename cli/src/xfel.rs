use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use color_eyre::eyre::anyhow;
use color_eyre::Result;
use serde::Deserialize;
use wait_timeout::ChildExt;

pub struct XfelDevice {
    flash: InternalFlash,
}

impl XfelDevice {
    pub fn connect(flash: InternalFlash) -> Result<XfelDevice> {
        let mut child = xfel_cmd().arg("version").spawn()?;
        if let Some(status) = child.wait_timeout(Duration::from_millis(500))? {
            if !status.success() {
                return Err(anyhow!("xfel version failed"));
            }
        } else {
            return Err(anyhow!("xfel version failed"));
        }
        let mut cmd = xfel_cmd();
        match flash {
            InternalFlash::SpiNor => cmd.arg("spinor"),
            InternalFlash::SpiNand => cmd.arg("spinand"),
        };
        let mut child = cmd.spawn()?;
        if let Some(status) = child.wait_timeout(Duration::from_millis(500))? {
            if !status.success() {
                return Err(anyhow!("xfel flash detect failed"));
            }
        } else {
            return Err(anyhow!("incorrect flash specified"));
        }
        Ok(XfelDevice { flash })
    }

    pub fn write_flash(&self, addr: usize, path: &Path) -> Result<()> {
        let mut cmd = xfel_cmd();
        match self.flash {
            InternalFlash::SpiNor => cmd.arg("spinor"),
            InternalFlash::SpiNand => cmd.arg("spinand"),
        };
        cmd.arg("write")
            .arg(format!("{}", addr))
            .arg(path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()?;
        Ok(())
    }

    pub fn reset(&self) -> Result<()> {
        Command::new("xfel").arg("reset").output()?;
        // NOTE: not handling errors here since xfel errors on reset even on success
        Ok(())
    }
}

fn xfel_cmd() -> Command {
    Command::new("xfel")
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub enum InternalFlash {
    SpiNor,
    SpiNand,
}

impl FromStr for InternalFlash {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nor" => Ok(InternalFlash::SpiNor),
            "nand" => Ok(InternalFlash::SpiNand),
            _ => Err("invalid flash type"),
        }
    }
}
