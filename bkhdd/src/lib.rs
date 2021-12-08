use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use binrw::{binrw, BinRead};
use byteordered::byteorder::ReadBytesExt;
use byteordered::ByteOrdered;
use io::BinInvertedReader;
use thiserror::Error;

pub mod io;

#[derive(Error, Debug)]
pub enum AHDDError {
    #[error("Can't get fh as mut")]
    FhMut,
    #[error("Can't get fh ref")]
    FhRef,
    #[error("File name is not set")]
    EmptyName,
    #[error("Header read error size {} <=> {0}", BLOCK_SIZE)]
    ReadHeaderSize(usize),
    #[error("Io Error")] //
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("BinRW Error")]
    BinRW {
        #[from]
        source: binrw::Error,
    },
    #[error("Uknown Error")]
    Unknown,
}

fn swap_pairs_adaptor<'a, T: 'a>(slice: &'a [T]) -> impl Iterator<Item = T> + 'a
where
    T: Clone + Copy,
{
    slice
        .chunks(2)
        .flat_map(|i| {
            if i.len() == 2 {
                [Some(i[1]), Some(i[0])]
            } else {
                [Some(i[0]), None]
            }
        })
        .flatten()
}

pub fn swap_pairs<'a, T: 'a>(slice: &'a [T]) -> Vec<T>
where
    T: Clone + Copy,
{
    swap_pairs_adaptor(slice).collect::<Vec<_>>()
}

pub const BLOCK_SIZE: usize = 512;

///
/// Константы взяты из HDIStuff.h
/// константы для доступа к данным таблицы разделов АльтПро
///
/// номер блока, где находится таблица разделов АльтПро
pub const AHDD_PT_SEC: usize = 7;
/// смещение к первой записи таблицы разделов
pub const AHDD_PART_B: usize = 0o766;
/// u8 количество логических дисков разделов
pub const AHDD_LOGD_B: usize = 0o770;
/// u8 хуй знает
pub const AHDD_UNI_B: usize = 0o771;
/// u16 количество секторов на дорожке?
pub const AHDD_SEC_B: usize = 0o772;
/// u8 количество головок?
pub const AHDD_HEAD_B: usize = 0o774;
/// u8 номер загрузозного раздела (LD)?
pub const AHDD_DRV_B: usize = 0o775;
/// u16 количество цидиндров
pub const AHDD_CYL_B: usize = 0o776;
/// u16 инициализация для контрольной суммы
pub const AHDD_CS_INIT: u16 = 0o12701;

/// Altec Pro HDD Layout
#[binrw]
#[brw(little)]
#[derive(Default, Debug)]
pub struct AHDDLayout {
    /// u8 количество логических дисков разделов
    partitions: u8, // 0o770
    /// u8 хуй знает
    uni: u8, // 0o771
    /// u16 количество секторов на дорожке?
    sectors: u16, // 0o772
    /// u8 количество головок (дорожек)?
    heads: u8, // 0o774
    /// u8 номер загрузозного раздела (LD)?
    drv: u8, // 0o775
    /// u16 количество цидиндров
    cylinders: u16, // 0o776
}

pub struct AHDD {
    file_name: String,
    fh: Option<fs::File>,
    read_only: bool,
    offset: u64,
    partitions: Vec<Partition>,
    checksum: u16,
    layout: AHDDLayout,
    raw: [u8; BLOCK_SIZE],
}

impl Default for AHDD {
    fn default() -> Self {
        Self {
            file_name: Default::default(),
            fh: None,
            read_only: true,
            offset: 0,
            partitions: Vec::new(),
            checksum: AHDD_CS_INIT,
            layout: Default::default(),
            raw: [0u8; BLOCK_SIZE],
        }
    }
}

#[derive(Debug, Default)]
struct Partition {
    start_cylinder: u16,
    start_head: u16,
    start_sector: u16,
    lba: u32,
    length: u32,
    end_cylinder: u16,
    end_head: u16,
    end_sector: u16,
    protected: bool,
}

impl AHDD {
    pub fn new(fname: &str) -> Self {
        Self {
            file_name: String::from(fname),
            ..Default::default()
        }
    }

    pub fn open(&mut self) -> Result<(), AHDDError> {
        if self.file_name.is_empty() {
            return Err(AHDDError::EmptyName);
        }

        let fh = OpenOptions::new()
            .read(true)
            .write(!self.read_only)
            .append(false)
            .open(&self.file_name)?;

        self.fh = Some(fh);

        Ok(())
    }

    pub fn fh_mut(&mut self) -> Result<&mut fs::File, AHDDError> {
        if let Some(fh) = self.fh.as_mut() {
            Ok(fh)
        } else {
            Err(AHDDError::FhMut)
        }
    }

    pub fn fh_ref(&mut self) -> Result<&fs::File, AHDDError> {
        if let Some(fh) = self.fh.as_ref() {
            Ok(fh)
        } else {
            Err(AHDDError::FhRef)
        }
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.offset = offset;
    }

    pub fn read_header(&mut self) -> Result<(), AHDDError> {
        if self.fh.is_none() {
            self.open()?
        }
        if let Some(fh) = self.fh.as_mut() {
            let mut reader = BinInvertedReader::new(fh);
            let mut buf = vec![0u8; BLOCK_SIZE];
            let offset = self.offset + (AHDD_PT_SEC * BLOCK_SIZE) as u64;
            let pos = reader.seek(SeekFrom::Start(offset))?;
            // dbg!(&pos);
            let size = reader.read(&mut buf)?;
            // eprintln!("buf {:x?}", buf);
            if size != BLOCK_SIZE {
                return Err(AHDDError::ReadHeaderSize(size));
            }
            let mut c = Cursor::new(&buf[..]);
            let pos = c.seek(SeekFrom::Start(AHDD_LOGD_B as u64))?;
            // dbg!(&pos);
            let layout = AHDDLayout::read(&mut c)?;
            // dbg!(&layout);
            let pos = c.seek(SeekFrom::Start(
                AHDD_PART_B as u64 - (4 * layout.partitions as u64),
            ))?;
            // dbg!(&pos);
            let mut br = ByteOrdered::le(&mut c);
            let checksum = br.read_u16()?;
            // dbg!(&checksum);
            for _ in 0..layout.partitions {
                // dbg!(&br.stream_position());
                let mut part = Partition::default();
                // размер раздела в блоках
                let len = br.read_u16()? as u32;
                part.length = len;
                // номер первого цилинда и номер головки
                let mut cyl = br.read_u16()?;
                if (cyl as i16) < 0 {
                    part.protected = true;
                    cyl = !cyl;
                }
                let head = cyl & 0xF;
                cyl >>= 4;
                // рассчитываем начало раздела в блоках
                let lba: u32 =
                    (cyl as u32 * layout.heads as u32 + head as u32) * layout.sectors as u32;
                part.lba = lba;
                part.start_cylinder = cyl;
                part.start_head = head;
                part.start_sector = 1; // всегда 1
                                       // конец раздела
                let end = lba + len;
                part.end_cylinder = (end / (layout.heads as u16 * layout.sectors) as u32) as u16;
                part.end_head = ((end / layout.sectors as u32) % layout.heads as u32) as u16;
                part.end_sector = (end % layout.sectors as u32 + 1) as u16;

                self.partitions.insert(0, part);
            }
            dbg!(&self.partitions);
            // dbg!(&br.stream_position());
            let offset = AHDD_PART_B as u64 - (4 * layout.partitions as u64) + 2;
            // dbg!(&offset);
            let mut cs = AHDD_CS_INIT;
            let mut count = 0;
            for i in (0..(BLOCK_SIZE as u64 - offset) / 2) {
                let pos = br.seek(SeekFrom::Start(BLOCK_SIZE as u64 - (i * 2) - 2))?;
                // dbg!(&pos);
                cs = cs.wrapping_add(br.read_u16()?);
                count += 1;
            }
            // dbg!(&checksum, &cs);
            // let pos = br.seek(SeekFrom::Start(offset - 2))?;
            // dbg!(&pos);
            // let crc = br.read_u16()?;
            // dbg!(&crc, &count);
            // cs.wrapping_add(crc);
            // dbg!(&cs);
            // dbg!(&c.stream_position());
            assert!(checksum == cs);
            self.checksum = cs;
            let part = &self.partitions[0];
            let pos = part.lba;
            dbg!(&pos);
            // let offset =
            //     self.offset + (BLOCK_SIZE * AHDD_PT_SEC) as u64 + (pos as u64 * BLOCK_SIZE as u64);
            let offset = self.offset + (pos as u64 * BLOCK_SIZE as u64);
            // let fh = reader.into_inner();
            let pos1 = reader.seek(SeekFrom::Start(offset))?;
            dbg!(&pos1);
            let size = reader.read(&mut buf)?;
            eprintln!("buf {:x?}", buf);
            if size != BLOCK_SIZE {
                return Err(AHDDError::ReadHeaderSize(size));
            }
            // eprintln!("{:o} ", b);
            let mut w = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .append(false)
                .open("test_block.dump")?;
            w.write(&buf[..])?;
            w.flush()?;
        } else {
            return Err(AHDDError::FhMut);
        }

        Ok(())
    }

    pub fn checksum(&self) {}
}

///
/// константы для доступа к данным таблицы разделов Самара
///
/// номер блока, где находится таблица разделов Самара
pub const SHDD_PT_SEC: usize = 1;
/// # устр. для загрузки по умолч. (0 - А, 2 - С ...)
pub const SHDD_BOOT_W: usize = 0;
/// объём цилиндра (общее количество секторов на дорожке) == H * S
pub const SHDD_CYLVOL_W: usize = 1;
/// количество секторов на дорожке & номер последней головки (H - 1)
pub const SHDD_HEAD_SEC_W: usize = 2;
/// таблица разделов
pub const SHDD_PART_W: usize = 3;
//
/// # устр. для загрузки по умолч. (0 - А, 2 - С ...)
pub const SHDD_BOOT_B: usize = 0;
/// объём цилиндра (общее количество секторов на дорожке) == H * S
pub const SHDD_CYLVOL_B: usize = 2;
/// количество секторов на дорожке
pub const SHDD_SEC_B: usize = 4;
/// номер последней головки (H - 1)
pub const SHDD_HEAD_B: usize = 5;
/// таблица разделов
pub const SHDD_PART_B: usize = 6;

///
/// константы для доступа к данным начального блока раздела Самара
///
/// номер лог. диска
pub const SHDD_NLD_W: usize = 0;
/// размер лог. диска в блоках
pub const SHDD_LD_LEN_W: usize = 1;
/// флаги - признаки
pub const SHDD_LD_FLAGS_W: usize = 2;
/// адрес загрузки загрузчика лог. диска
pub const SHDD_ADR_BOOT_W: usize = 3;
/// адрес блока параметров для загрузчика
pub const SHHD_ADR_PAR_W: usize = 4;
/// состояние регистра страниц
pub const SHDD_PAGE_W: usize = 5;

/// HDI layout
#[binrw]
#[brw(little)]
pub struct HDILayout {
    main_config: u16,            // 0
    cylinders: u16,              // 1
    word2: u16,                  // 2
    heads: u16,                  // 3
    raw_bytes_per_track: u16,    // 4
    raw_bytes_per_sector: u16,   // 5
    sectors: u16,                // 6
    reserved7: [u16; 3],         // 7,8,9
    serial_number: [u8; 20],     // 10
    buffer_type: u16,            // 20
    buffer_size_in_sectors: u16, // 21
    ecc_bytes_num: u16,          // 22
    fw_version: [u8; 8],         // 23
    pub model_name: [u8; 40],    // 27
    word47: u16,                 // 47
    word48: u16,                 // 48
    capabilities1: u16,          // 49
    capabilities2: u16,          // 50
    reserved51: [u16; 6],        // 51
    capacity_in_sectors: u32,    // 57,58
    reserved59: u16,             // 59
    total_used_sectors: u32,     // 60,61
    reserved62: [u16; 193],      // 62
    checksum_magic: u8,          // 255 - must be 0a5
    checksum: u8,                // +1 b
}

impl std::fmt::Debug for HDILayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::ffi::CStr;

        f.debug_struct("HDILayout")
            .field("main_config", &format_args!("{:x}", &self.main_config))
            .field("cylinders", &self.cylinders)
            .field("word2", &format_args!("{:x}", &self.word2))
            .field("heads", &self.heads)
            .field("raw_bytes_per_track", &self.raw_bytes_per_track)
            .field("raw_bytes_per_sector", &self.raw_bytes_per_sector)
            .field("sectors", &self.sectors)
            .field("reserved7", &format_args!("{:x?}", &self.reserved7))
            .field(
                "serial_number",
                &format_args!(
                    "{:?} ({:?})",
                    &String::from_utf8_lossy(&self.serial_number).trim_end(),
                    &String::from_utf8_lossy(&swap_pairs(&self.serial_number)).trim_end(),
                ),
            )
            .field("buffer_type", &self.buffer_type)
            .field("buffer_size_in_sectors", &self.buffer_size_in_sectors)
            .field("ecc_bytes_num", &self.ecc_bytes_num)
            .field(
                "fw_version",
                &format_args!(
                    "{:?} ({:?})",
                    &String::from_utf8_lossy(&self.fw_version).trim_end(),
                    &String::from_utf8_lossy(&swap_pairs(&self.fw_version)).trim_end(),
                ),
                // &CStr::from_bytes_with_nul(&self.fw_version).unwrap_or_default(),
            )
            .field(
                "model_name",
                &format_args!(
                    "{:?} ({:?})",
                    &String::from_utf8_lossy(&self.model_name).trim_end(),
                    &String::from_utf8_lossy(&swap_pairs(&self.model_name)).trim_end(),
                ),
                // &CStr::from_bytes_with_nul(&self.model_name).unwrap_or_default(),
            )
            .field("word47", &format_args!("{:x}", &self.word47))
            .field("word48", &self.word48)
            .field("capabilities1", &format_args!("{:x}", &self.capabilities1))
            .field("capabilities2", &format_args!("{:x}", &self.capabilities2))
            .field("reserved51", &format_args!("{:x?}", &self.reserved51))
            .field("capacity_in_sectors", &self.capacity_in_sectors)
            .field("reserved59", &self.reserved59)
            .field("total_used_sectors", &self.total_used_sectors)
            // .field("reserved62", &self.reserved62)
            .field("reserved62", &format_args!("{:x?}", &self.reserved62))
            .field(
                "checksum_magic",
                &format_args!("{:x?}", &self.checksum_magic),
            )
            .field("checksum", &format_args!("{:x?}", &self.checksum))
            .finish()
    }
}

impl Default for HDILayout {
    fn default() -> Self {
        Self {
            main_config: 0x045a,       // 0
            cylinders: 0,              // 1
            word2: 0xc837,             // 2
            heads: 0,                  // 3
            raw_bytes_per_track: 0,    // 4
            raw_bytes_per_sector: 0,   // 5
            sectors: 0,                // 6
            reserved7: [0; 3],         // 7,8,9
            serial_number: [0; 20],    // 10
            buffer_type: 1,            // 20
            buffer_size_in_sectors: 1, // 21
            ecc_bytes_num: 4,          // 22
            fw_version: [0; 8],        // 23
            model_name: [0; 40],       // 27
            word47: 0x8001,            // 47
            word48: 0,                 // 48
            capabilities1: 0x200,      // 49
            capabilities2: 0x4000,     // 50
            reserved51: [0; 6],        // 51
            capacity_in_sectors: 0,    // 57,58
            reserved59: 0,             // 59
            total_used_sectors: 0,     // 60,61
            reserved62: [0; 193],      // 62
            checksum_magic: 0,         // 254
            checksum: 0,               // + 1b
        }
    }
}

/// Main HDI Struct
pub struct HDI {
    file_name: String,
    meta: HDILayout,
    raw: [u8; BLOCK_SIZE],
}

impl Default for HDI {
    fn default() -> Self {
        Self {
            file_name: Default::default(),
            meta: HDILayout::default(),
            raw: [0u8; BLOCK_SIZE],
        }
    }
}

impl HDI {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn checksum(&self) -> u8 {
        let cs = self.raw[..(BLOCK_SIZE - 1)]
            .iter()
            .fold(0u8, |sum, &b| sum.wrapping_add(b));
        -(cs as i8) as u8
    }
}
