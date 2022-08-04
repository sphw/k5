use core::ops::Range;

use defmt::Format;
use enumflags2::{bitflags, BitFlags};

use crate::KernelError;

#[derive(Clone, Default)]
pub struct RegionTable {
    pub regions: heapless::Vec<Region, 8>,
}

#[allow(dead_code)]
impl RegionTable {
    pub fn push(&mut self, region: Region) -> Result<(), KernelError> {
        let mut i = 0;
        let mut inserted_at = None;
        while i < self.regions.len() {
            if self.regions[i] == region {
                i += 1;
                continue;
            }
            if inserted_at.map(|x| i > x + 2).unwrap_or_default() {
                return Ok(());
            }
            let mut old_end = None;

            if self.regions[i].range.contains(&region.range.start) {
                if self.regions[i].range.contains(&region.range.end)
                    && self.regions[i].attr.contains(region.attr)
                {
                    return Ok(());
                }
                inserted_at = Some(i);
                old_end = Some(self.regions[i].range.end);
                self.regions[i].range.end = region.range.start; // TODO: Handle case where region.start == new_region.start
            }
            if let Some(end) = old_end {
                if (self.regions[i].range.start..end).contains(&region.range.end) {
                    self.regions
                        .insert(
                            i + 1,
                            Region {
                                range: region.range.end..end,
                                attr: self.regions[i].attr,
                            },
                        )
                        .map_err(|_| KernelError::ABI(abi::Error::BufferOverflow))?;
                }
            } else if self.regions[i].range.contains(&region.range.end) {
                self.regions[i].range.start = region.range.end; // TODO: Handle case where region.start == new_region.start
            }
            if inserted_at.is_none()
                && region.range.start > self.regions[i].range.end
                && ((i == self.regions.len() - 1)
                    || region.range.end < self.regions[i + 1].range.start)
            {
                inserted_at = Some(i);
            }
            if inserted_at == Some(i) {
                self.regions
                    .insert(i + 1, region.clone())
                    .map_err(|_| KernelError::ABI(abi::Error::BufferOverflow))?;
            }
            i += 1;
        }
        Ok(())
    }

    pub fn pop(&mut self, region: Region) {
        let mut i = 0;
        while i < self.regions.len() {
            if self.regions[i] == region {
                if i > 0
                    && i < self.regions.len() - 1
                    && self.regions[i - 1].range.end == region.range.start
                    && self.regions[i + 1].range.start == region.range.end
                    && self.regions[i - 1].attr == self.regions[i + 1].attr
                {
                    self.regions[i - 1].range.end = self.regions[i + 1].range.end;
                    self.regions.remove(i + 1);
                }
                self.regions.remove(i);
                return;
            }
            i += 1;
        }
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Region {
    pub range: Range<usize>,
    pub attr: BitFlags<RegionAttr>,
}

#[bitflags]
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegionAttr {
    Write,
    Read,
    Exec,
    Device,
    Dma,
}

#[cfg(test)]
mod tests {
    use crate::regions::*;

    #[test]
    fn test_insert_region() {
        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[Region {
                range: 0..200,
                attr: Default::default(),
            }])
            .unwrap(),
        };
        table
            .push(Region {
                range: 20..50,
                attr: RegionAttr::Write.into(),
            })
            .unwrap();
        assert_eq!(
            table.regions.as_slice(),
            &[
                Region {
                    range: 0..20,
                    attr: Default::default(),
                },
                Region {
                    range: 20..50,
                    attr: RegionAttr::Write.into(),
                },
                Region {
                    range: 50..200,
                    attr: Default::default(),
                }
            ]
        );
        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[Region {
                range: 0..30,
                attr: Default::default(),
            }])
            .unwrap(),
        };
        table
            .push(Region {
                range: 20..50,
                attr: RegionAttr::Write.into(),
            })
            .unwrap();
        assert_eq!(
            table.regions.as_slice(),
            &[
                Region {
                    range: 0..20,
                    attr: Default::default(),
                },
                Region {
                    range: 20..50,
                    attr: RegionAttr::Write.into(),
                },
            ]
        );
        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[
                Region {
                    range: 0..50,
                    attr: Default::default(),
                },
                Region {
                    range: 50..100,
                    attr: Default::default(),
                },
            ])
            .unwrap(),
        };
        table
            .push(Region {
                range: 20..60,
                attr: RegionAttr::Write.into(),
            })
            .unwrap();
        assert_eq!(
            table.regions.as_slice(),
            &[
                Region {
                    range: 0..20,
                    attr: Default::default(),
                },
                Region {
                    range: 20..60,
                    attr: RegionAttr::Write.into(),
                },
                Region {
                    range: 60..100,
                    attr: Default::default(),
                },
            ]
        );

        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[
                Region {
                    range: 0..50,
                    attr: Default::default(),
                },
                Region {
                    range: 90..100,
                    attr: Default::default(),
                },
            ])
            .unwrap(),
        };
        table
            .push(Region {
                range: 60..80,
                attr: RegionAttr::Write.into(),
            })
            .unwrap();
        assert_eq!(
            table.regions.as_slice(),
            &[
                Region {
                    range: 0..50,
                    attr: Default::default(),
                },
                Region {
                    range: 60..80,
                    attr: RegionAttr::Write.into(),
                },
                Region {
                    range: 90..100,
                    attr: Default::default(),
                },
            ]
        )
    }

    #[test]
    fn test_pop_region() {
        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[
                Region {
                    range: 0..20,
                    attr: Default::default(),
                },
                Region {
                    range: 20..50,
                    attr: RegionAttr::Write.into(),
                },
                Region {
                    range: 50..200,
                    attr: Default::default(),
                },
            ])
            .unwrap(),
        };
        table.pop(Region {
            range: 20..50,
            attr: RegionAttr::Write.into(),
        });
        assert_eq!(
            table.regions.as_slice(),
            &[Region {
                range: 0..200,
                attr: Default::default(),
            },]
        );

        let mut table = RegionTable {
            regions: heapless::Vec::from_slice(&[
                Region {
                    range: 0..10,
                    attr: Default::default(),
                },
                Region {
                    range: 20..50,
                    attr: RegionAttr::Write.into(),
                },
                Region {
                    range: 50..200,
                    attr: Default::default(),
                },
            ])
            .unwrap(),
        };
        table.pop(Region {
            range: 20..50,
            attr: RegionAttr::Write.into(),
        });
        assert_eq!(
            table.regions.as_slice(),
            &[
                Region {
                    range: 0..10,
                    attr: Default::default(),
                },
                Region {
                    range: 50..200,
                    attr: Default::default(),
                },
            ]
        );
    }
}
