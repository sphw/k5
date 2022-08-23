use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{build::Config, elf::Elf};
use color_eyre::{eyre::anyhow, Result};
use colored::Colorize;
use defmt_decoder::{DecodeError, Frame, Locations};
use probe_rs::{Core, MemoryInterface as _, Session};
use probe_rs_rtt::{Rtt, ScanRegion, UpChannel};
use serialport::{SerialPort, SerialPortType};
use signal_hook::consts::signal;

const TIMEOUT: Duration = Duration::from_secs(2);

fn attach_rtt(elf: &Elf, session: &mut Session) -> Result<UpChannel> {
    let mem_map = session.target().memory_map.clone();
    let mut core = session.core(0)?;
    let rtt_buffer_address = elf
        .rtt_buffer_address()
        .ok_or_else(|| anyhow!("rtt buffer not available"))?;
    let scan_region = ScanRegion::Exact(rtt_buffer_address);
    for _ in 0..50 {
        let mut rtt = match Rtt::attach_region(&mut core, &mem_map, &scan_region) {
            Err(probe_rs_rtt::Error::ControlBlockNotFound) => continue,
            rtt => rtt?,
        };
        let channel = rtt
            .up_channels()
            .take(0)
            .ok_or_else(|| anyhow!("RTT up channel 0 not found"))?;
        return Ok(channel);
    }
    Err(anyhow!("failed to attach rtt"))
}

pub fn print_logs(config: &Config, kernel_path: PathBuf, source: LogSource) -> Result<()> {
    let mut elf = fs::File::open(kernel_path)?;
    let mut elf_data = vec![];
    elf.read_to_end(&mut elf_data)?;
    let task_name_width = config
        .tasks
        .iter()
        .map(|t| t.name.len())
        .max()
        .unwrap_or_default()
        + 2;
    let mut task_elf_data = config
        .tasks
        .iter()
        .map(|t| {
            let path = t.target_dir().join("final.elf");
            let mut elf = fs::File::open(path)?;
            let mut elf_data = vec![];
            elf.read_to_end(&mut elf_data)?;
            Ok(elf_data)
        })
        .collect::<Result<Vec<_>>>()?;
    task_elf_data.insert(0, elf_data.clone());
    let task_elfs = task_elf_data
        .iter()
        .map(|elf_data| {
            let elf = crate::elf::Elf::parse(elf_data).map_err(|e| anyhow!(e))?;
            Ok(elf)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut task_decoders = task_elfs
        .iter()
        .map(|elf| {
            let table = elf
                .defmt_table
                .as_ref()
                .ok_or_else(|| anyhow!("missing defmt table for task"))?;
            Ok(table.new_stream_decoder())
        })
        .collect::<Result<Vec<_>>>()?;
    let mut task_names: Vec<_> = config.tasks.iter().map(|t| t.name.clone()).collect();
    task_names.insert(0, "kern".to_string());
    let elf = Elf::parse(&elf_data).map_err(|e| anyhow!(e))?;
    let mut log_session = source.attach(&elf)?;
    let mut was_halted = false;
    let current_dir = std::env::current_dir().unwrap();
    let exit = Arc::new(AtomicBool::new(false));
    let sig_id = signal_hook::flag::register(signal::SIGINT, exit.clone())?;
    let mut reader = LogReader::default();
    reader.find_start_marker(&mut log_session)?;
    while !exit.load(Ordering::Relaxed) {
        reader.read(&mut log_session)?;
        while let Some((task_id, buf)) = reader.frame()? {
            let elf = &task_elfs[task_id];
            let task_name = &task_names[task_id];
            let decoder = &mut task_decoders[task_id];
            decoder.received(&buf);
            loop {
                match decoder.decode() {
                    Ok(frame) => {
                        println!(
                            "{}{} {}",
                            level_string(frame.level()),
                            format!("{:^fill$}", task_name, fill = task_name_width)
                                .bold()
                                .white()
                                .on_truecolor(0, 142, 245),
                            frame.display_message()
                        );
                        if let Some(locs) = &elf.defmt_locations {
                            let (path, line, module) = location_info(&frame, locs, &current_dir);
                            print_location(&path, line, &module)?;
                        }
                    }
                    Err(DecodeError::UnexpectedEof) => {
                        break;
                    }
                    Err(DecodeError::Malformed) => {
                        break;
                    }
                }
            }
        }
        let is_halted = log_session.was_halted()?;

        if is_halted && was_halted {
            break;
        }
        was_halted = is_halted;
    }
    signal_hook::low_level::unregister(sig_id);
    signal_hook::flag::register_conditional_default(signal::SIGINT, exit.clone())?;
    if exit.load(Ordering::Relaxed) {
        log_session.halt()?;
    }

    Ok(())
}

fn start_program(sess: &mut Session, elf: &Elf) -> Result<()> {
    let mut core = sess.core(0)?;

    if let Some(rtt_buffer_address) = elf.rtt_buffer_address() {
        set_rtt_to_blocking(&mut core, elf.main_fn_address(), rtt_buffer_address)?
    }

    //core.set_hw_breakpoint(cortexm::clear_thumb_bit(elf.vector_table.hard_fault) as u64)?;
    core.run()?;

    Ok(())
}

/// Set rtt to blocking mode
fn set_rtt_to_blocking(
    core: &mut Core,
    main_fn_address: u32,
    rtt_buffer_address: u32,
) -> Result<()> {
    // set and wait for a hardware breakpoint at the beginning of `fn main()`
    core.set_hw_breakpoint(main_fn_address as u64)?;
    core.run()?;
    core.wait_for_core_halted(Duration::from_secs(5))?;

    // calculate address of up-channel-flags inside the rtt control block
    const OFFSET: u32 = 44;
    let rtt_buffer_address = rtt_buffer_address + OFFSET;

    // read flags
    let channel_flags = &mut [0];
    core.read_32(rtt_buffer_address as u64, channel_flags)?;
    // modify flags to blocking
    const MODE_MASK: u32 = 0b11;
    const MODE_BLOCK_IF_FULL: u32 = 0b10;
    let modified_channel_flags = (channel_flags[0] & !MODE_MASK) | MODE_BLOCK_IF_FULL;
    // write flags back
    core.write_word_32(rtt_buffer_address as u64, modified_channel_flags)?;

    // clear the breakpoint we set before
    core.clear_hw_breakpoint(main_fn_address as u64)?;

    Ok(())
}

fn print_location(file: &str, line: u32, module_path: &str) -> io::Result<()> {
    let mod_path = module_path;
    let loc = format!("{}:{}", file, line);
    println!("{}", format!("└─ {} @ {}", mod_path, loc).dimmed());
    Ok(())
}

fn level_string(level: Option<defmt_parser::Level>) -> colored::ColoredString {
    use defmt_parser::Level;
    match level {
        Some(level) => match level {
            Level::Debug => " debug ".bold().white().on_truecolor(97, 97, 97),
            Level::Trace => " trace ".bold().white().on_truecolor(97, 97, 97),
            Level::Info => " info  ".bold().white().on_truecolor(121, 199, 255),
            Level::Error => " error ".bold().white().on_red(),
            Level::Warn => " warn  ".bold().white().on_yellow(),
        },
        None => " print ".bold().white().on_truecolor(97, 97, 97),
    }
}

fn location_info(
    frame: &Frame,
    locations: &Locations,
    current_dir: &Path,
) -> (String, u32, String) {
    let location = &locations[&frame.index()];
    let path = if let Some(relpath) = pathdiff::diff_paths(&location.file, current_dir) {
        relpath.display().to_string()
    } else {
        location.file.display().to_string()
    };
    (path, location.line as u32, location.module.clone())
}

pub enum LogSource<'a> {
    Rtt(&'a mut Session),
    Serial,
}

impl<'a> LogSource<'a> {
    fn attach(self, elf: &Elf) -> Result<LogSession<'a>> {
        match self {
            LogSource::Rtt(session) => {
                session.core(0).unwrap().reset_and_halt(TIMEOUT)?;
                start_program(session, &elf)?;
                let channel = attach_rtt(&elf, session)?;
                let core = session.core(0)?;
                Ok(LogSession::Rtt { core, channel })
            }
            LogSource::Serial => {
                let port = serialport::available_ports()?
                    .into_iter()
                    .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
                    .next()
                    .ok_or_else(|| anyhow!("no serial port found"))?;
                println!("attaching to port: {:?}", port.port_name);
                let port = serialport::new(port.port_name, 115200)
                    .timeout(Duration::from_secs(60))
                    .open()?;
                Ok(LogSession::Serial(port))
            }
        }
    }
}

enum LogSession<'a> {
    Rtt {
        core: probe_rs::Core<'a>,
        channel: UpChannel,
    },
    Serial(Box<dyn SerialPort>),
}

impl<'a> LogSession<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            LogSession::Rtt {
                ref mut core,
                channel,
            } => Ok(channel.read(core, buf)?),
            LogSession::Serial(port) => Ok(port.read(buf)?),
        }
    }

    fn halt(&mut self) -> Result<()> {
        match self {
            LogSession::Rtt { ref mut core, .. } => {
                core.halt(TIMEOUT)?;
                Ok(())
            }
            LogSession::Serial(_) => Ok(()),
        }
    }

    fn was_halted(&mut self) -> Result<bool> {
        match self {
            LogSession::Rtt { ref mut core, .. } => {
                let halted = core.core_halted()?;
                Ok(halted)
            }
            LogSession::Serial(_) => Ok(false),
        }
    }
}

#[derive(Default)]
struct LogReader {
    buf: Vec<u8>,
    current_frame_length: Option<usize>,
}
impl LogReader {
    fn find_start_marker(&mut self, log: &mut LogSession) -> Result<()> {
        if matches!(log, LogSession::Rtt { .. }) {
            return Ok(());
        }
        let mut search_buf = vec![];
        let mut read_buf = [0u8; 1024];
        let mut current_pos = 0;
        loop {
            let len = log.read(&mut read_buf)?;
            search_buf.extend_from_slice(&read_buf[..len]);
            if current_pos + 9 < search_buf.len() {
                if &search_buf[current_pos..current_pos + 9] == b"LOG_START" {
                    if current_pos + 9 < search_buf.len() {
                        self.buf.extend_from_slice(&search_buf[current_pos + 9..]);
                    }
                    return Ok(());
                }
                current_pos += 1;
            }
        }
    }

    fn read(&mut self, log: &mut LogSession) -> Result<()> {
        let mut buf = [0u8; 1024];
        let len = log.read(&mut buf)?;
        self.buf.extend_from_slice(&buf[..len]);
        Ok(())
    }

    fn frame(&mut self) -> Result<Option<(usize, Vec<u8>)>> {
        if self.current_frame_length.is_none() && self.buf.len() >= 2 {
            self.current_frame_length = Some(self.buf[0] as usize);
        }
        if let Some(length) = self.current_frame_length {
            if length + 1 < self.buf.len() {
                let mut frame = self.buf.split_off(length + 1);
                std::mem::swap(&mut self.buf, &mut frame);
                self.current_frame_length = None;
                return Ok(Some((frame[1] as usize, frame[2..].to_vec())));
            }
        }
        Ok(None)
    }
}
