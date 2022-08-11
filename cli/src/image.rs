use color_eyre::eyre::anyhow;
use color_eyre::Result;
use std::{
    collections::HashMap,
    fs::{self},
    path::Path,
    process::Command,
};

pub use egon::*;

use crate::build::{
    align_up, get_elf_size, Kernel, MemoryRole, MemorySection, Platform, SRecWriter, Task, TaskLoc,
    TASK_LINK_BYTES,
};

pub(crate) trait ImageBuilder {
    type Image;
    fn task(&mut self, task: &Task) -> Result<()>;
    fn kernel(&mut self, kern: &Kernel) -> Result<()>;
    fn build(&mut self) -> Result<Self::Image>;
}

pub trait Image {
    fn write(&self, target_path: &Path) -> Result<()>;
}

pub struct SRecImageBuilder {
    current_locs: HashMap<String, MemorySection>,
    regions: HashMap<String, MemorySection>,
    platform: Platform,
    codegen_tasks: Vec<codegen::Task>,
    output: SRecWriter,
}

impl SRecImageBuilder {
    pub(crate) fn new(
        regions: HashMap<String, MemorySection>,
        platform: Platform,
        kern: &Kernel,
    ) -> Self {
        let mut current_locs = regions.clone();
        for (name, size) in kern.sizes.iter() {
            let loc = &mut current_locs.get_mut(name).unwrap();
            loc.address += size;
        }
        Self {
            current_locs,
            regions,
            platform,
            codegen_tasks: vec![],
            output: SRecWriter::default(),
        }
    }
}

impl ImageBuilder for SRecImageBuilder {
    type Image = SRecImage;

    fn kernel(&mut self, kern: &Kernel) -> Result<()> {
        let kernel_path = kern.build(
            self.platform,
            self.regions.clone(),
            self.codegen_tasks.clone(),
        )?;
        self.output.write(&kernel_path)?;
        fs::copy(
            &kernel_path,
            kern.crate_path.join("target").join("kernel.elf"),
        )?;
        Ok(())
    }

    fn task(&mut self, task: &Task) -> Result<()> {
        let reloc = task.build()?;
        let elf = &task.target_dir().join("size.elf");
        task.link(
            &reloc,
            elf,
            &TaskLoc {
                regions: self.regions.clone(),
            },
            TASK_LINK_BYTES,
        )?;
        let sizes = get_elf_size(elf, &self.regions, task.stack_space_size)?;
        let regions: HashMap<_, _> = sizes
            .clone()
            .into_iter()
            .map(|(name, range)| {
                (
                    name.clone(),
                    MemorySection {
                        size: align_up(range.len(), 32),
                        ..self.current_locs[&name]
                    },
                )
            })
            .collect();
        for (name, size) in sizes.iter() {
            let loc = &mut self.current_locs.get_mut(name).unwrap();
            loc.address += align_up(size.len(), 32);
        }
        println!("{:?}", self.current_locs);
        let elf = task.target_dir().join("final.elf");
        task.link(
            &reloc,
            &elf,
            &TaskLoc {
                regions: regions.clone(),
            },
            TASK_LINK_BYTES,
        )?;

        let entrypoint = self.output.write(&elf)?;
        let stack_region = regions
            .values()
            .find(|r| r.role == MemoryRole::Stack)
            .ok_or_else(|| {
                anyhow!("no stack region found. Make sure to specify a signle region for the stack")
            })?;
        self.codegen_tasks.push(codegen::Task {
            name: task.name.clone(),
            entrypoint,
            stack_space: stack_region.address..stack_region.address + task.stack_space_size,
            init_stack_size: task.stack_size,
            regions: regions
                .values()
                .map(|r| r.address..r.address + r.size)
                .collect(),
        });
        Ok(())
    }

    fn build(&mut self) -> Result<Self::Image> {
        Ok(Self::Image {
            srec: self.output.finalize(),
        })
    }
}

pub struct SRecImage {
    srec: String,
}
impl Image for SRecImage {
    fn write(&self, target_path: &Path) -> Result<()> {
        let out_path = target_path.join("final.srec");
        fs::write(&out_path, &self.srec)?;
        let ihex_path = target_path.join("final.ihex");
        let output = Command::new("arm-none-eabi-objcopy")
            .arg("-Isrec")
            .arg(&out_path)
            .arg(ihex_path)
            .arg("-Oihex")
            .output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "objcopy failed: {:?}",
                std::str::from_utf8(&output.stderr)
            ));
        }
        let bin_path = target_path.join("final.bin");
        let output = Command::new("arm-none-eabi-objcopy")
            .arg("-Isrec")
            .arg(&out_path)
            .arg(bin_path)
            .arg("-Obinary")
            .output()?;
        if !output.status.success() {
            return Err(anyhow!(
                "objcopy failed: {:?}",
                std::str::from_utf8(&output.stderr)
            ));
        }
        Ok(())
    }
}

mod egon {
    //! An implementation of the egon header format for Allwinner SOCs
    //! NOTE: this is broken at the moment for unclear reasons, and needs to be fixed

    use super::{Image, ImageBuilder, SRecImage, SRecImageBuilder};
    use crate::build::{Kernel, MemorySection, Platform, Task};
    use bytemuck::{Pod, Zeroable};
    use byteorder::ReadBytesExt;
    use color_eyre::eyre::anyhow;
    use color_eyre::Result;
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Cursor;
    use std::mem;

    pub const D1_HEADER_SIZE: usize = core::mem::size_of::<HeadData>();

    pub struct D1ImageBuilder {
        flash_base_addr: usize,
        srec: SRecImageBuilder,
    }

    impl D1ImageBuilder {
        pub(crate) fn new(
            mut regions: HashMap<String, MemorySection>,
            platform: Platform,
            kern: &Kernel,
        ) -> Result<Self> {
            let flash_region = regions
                .get_mut("flash")
                .ok_or_else(|| anyhow!("flash region missing"))?;
            let flash_base_addr = flash_region.address;
            flash_region.address += D1_HEADER_SIZE;
            flash_region.size -= D1_HEADER_SIZE;
            Ok(Self {
                srec: SRecImageBuilder::new(regions, platform, kern),
                flash_base_addr,
            })
        }
    }

    const STAMP_CHECKSUM: u32 = 0x5F0A6C39;
    const EGON_MAGIC: [u8; 8] = *b"eGON.BT0";
    const DEFAULT_HEAD: HeadData = HeadData {
        jump_inst: RVJumpInst::new(mem::size_of::<HeadData>()),
        magic: EGON_MAGIC, // magic number
        checksum: STAMP_CHECKSUM,
        length: 0,
        pub_head_size: 0,
        fel_script_address: 0,
        fel_uenv_length: 0,
        dt_name_offset: 0,
        dram_size: 0,
        boot_media: 0,
        string_pool: [0; 13],
    };

    impl ImageBuilder for D1ImageBuilder {
        type Image = SRecImage;

        fn task(&mut self, task: &Task) -> Result<()> {
            self.srec.task(task)
        }

        fn kernel(&mut self, kern: &Kernel) -> Result<()> {
            self.srec.kernel(kern)
        }

        fn build(&mut self) -> Result<Self::Image> {
            let tmp_dir = tempdir::TempDir::new("d1-srec-temp")?;
            let image = self.srec.build()?;
            image.write(tmp_dir.path())?;
            let mut file = File::open(tmp_dir.path().join("final.bin"))?;
            //let length = align_up(file.metadata()?.len() as usize, 16 * 1024) as u32;
            let length = 32 * 1024;
            let mut head = HeadData {
                length,
                ..DEFAULT_HEAD
            };
            let mut checksum: u32 = 0;
            let mut head_cursor = Cursor::new(bytemuck::bytes_of(&head));
            checksum = calc_checksum(checksum, &mut head_cursor)?;
            file.set_len(32 * 1024);
            checksum = calc_checksum(checksum, &mut file)?;
            head.checksum = checksum;
            self.srec
                .output
                .write_slice(self.flash_base_addr, bytemuck::bytes_of(&head));
            self.srec.build()
        }
    }

    fn calc_checksum(mut checksum: u32, mut cursor: impl std::io::Read) -> Result<u32> {
        loop {
            match cursor.read_u32::<byteorder::LittleEndian>() {
                Ok(val) => checksum = checksum.wrapping_add(val),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(checksum),
                Err(err) => return Err(err.into()),
            }
        }
    }

    #[derive(Copy, Clone, Pod, Zeroable)]
    #[repr(C)]
    pub struct HeadData {
        jump_inst: RVJumpInst,
        magic: [u8; 8],
        checksum: u32,
        length: u32,
        pub_head_size: u32,
        fel_script_address: u32,
        fel_uenv_length: u32,
        dt_name_offset: u32,
        dram_size: u32,
        boot_media: u32,
        string_pool: [u32; 13],
    }

    #[derive(Copy, Clone, Pod, Zeroable)]
    #[repr(transparent)]
    struct RVJumpInst(u32);

    impl RVJumpInst {
        const fn new(addr: usize) -> Self {
            // let addr = addr as u32;
            // // source uboot:
            // // https://github.com/u-boot/u-boot/blob/aef6839747b5b01e3d1d32d16e712d42a6702b88/tools/sunxi_egon.c#L135
            // // basically generates a valid jump inst in rv64
            // let value = 0x0000006f
            //     | ((addr & 0x00100000) << 11)
            //     | ((addr & 0x000007fe) << 20)
            //     | ((addr & 0x00000800) << 9)
            //     | ((addr & 0x000ff000) << 0);
            // Self(value)
            Self(0x00_00_a0_85)
        }
    }
}
