use binrw::binrw;

pub const BLOCK_SIZE: usize = 512;

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
    model_name: [u8; 40],        // 27
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
        f.debug_struct("SysSector")
            .field("main_config", &self.main_config)
            .field("cylinders", &self.cylinders)
            .field("word2", &self.word2)
            .field("heads", &self.heads)
            .field("raw_bytes_per_track", &self.raw_bytes_per_track)
            .field("raw_bytes_per_sector", &self.raw_bytes_per_sector)
            .field("sectors", &self.sectors)
            .field("reserved7", &format_args!("{:x?}", &self.reserved7))
            .field(
                "serial_number",
                &CStr::from_bytes_with_nul(&self.serial_number).unwrap_or_default(),
            )
            .field("buffer_type", &self.buffer_type)
            .field("buffer_size_in_sectors", &self.buffer_size_in_sectors)
            .field("ecc_bytes_num", &self.ecc_bytes_num)
            .field(
                "fw_version",
                &CStr::from_bytes_with_nul(&self.fw_version).unwrap_or_default(),
            )
            .field(
                "model_name",
                &CStr::from_bytes_with_nul(&self.model_name).unwrap_or_default(),
            )
            .field("word47", &self.word47)
            .field("word48", &self.word48)
            .field("capabilities1", &self.capabilities1)
            .field("capabilities2", &self.capabilities2)
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
