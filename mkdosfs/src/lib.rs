use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use bytes::Buf;
use encoding_rs::KOI8_R;
use io::Reader;
use thiserror::Error;
use tracing::{debug, instrument, trace, warn};

pub mod io;

pub const BLOCK_SIZE: usize = 512;
pub const MKDOS_LABEL: u16 = 0o51414;
pub const MICRODOS_LABEL: u16 = 0o123456;
pub const DIR_MARKER: u8 = 0o177;
pub const DIR_ENTRY_SIZE: usize = 0o30;
pub const FILE_NAME_SIZE: usize = 14;
pub const META_SIZE: usize = 0o500;

#[derive(Debug, Copy, Clone)]
pub enum MetaOffset {
    Start = 0,
    /// 30 - Количество файлов в каталоге (НЕ ЗАПИСЕЙ!);
    Files = 0o30,
    /// 32 - Суммарное количество  блоков в файлах (НЕ ЗАПИСЯХ!) каталога;
    Blocks = 0o32,
    LabelsOffset = 0o400 - 0o34,
    /// 400 - Метка принадлежности к формату Micro DOS (123456);
    MicrodosLabel = 0o400,
    /// 402 - Метка формата каталога MK-DOS (51414);
    MkdosLabel = 0o402,
    DiskSizeOffset = 0o466 - 0o404,
    /// 466 - Размер диска в блоках, величина абсолютная для системы (в
    ///       отличие от NORD, NORTON и т.п.) принимающая не два значе-
    ///       ния под 40 или 80 дорожек, а, скажем, если  Ваш  дисковод
    ///       понимает только 76 дорожек, то в этой ячейке  нужно  ука-
    ///       зать соответствующее число блоков (это делается при  ини-
    ///       циализации);
    DiskSize = 0o466,
    /// 470 - Номер блока первого файла. Величина также изменяемая;
    StartBlock = 0o470,
    /// 500 - Первая запись о файле.
    DirEntriesStart = 0o500,
}

impl From<MetaOffset> for usize {
    fn from(off: MetaOffset) -> Self {
        off as usize
    }
}

#[derive(Copy, Clone)]
pub struct Meta {
    /// 30 - Количество файлов в каталоге (НЕ ЗАПИСЕЙ!);
    files: u16,
    /// 32 - Суммарное количество  блоков в файлах (НЕ ЗАПИСЯХ!) каталога;
    blocks: u16,
    /// 400 - Метка принадлежности к формату Micro DOS (123456);
    microdos_label: u16,
    /// 402 - Метка формата каталога MK-DOS (51414);
    mkdos_label: u16,
    /// 466 - Размер диска в блоках, величина абсолютная для системы (в
    ///       отличие от NORD, NORTON и т.п.) принимающая не два значе-
    ///       ния под 40 или 80 дорожек, а, скажем, если  Ваш  дисковод
    ///       понимает только 76 дорожек, то в этой ячейке  нужно  ука-
    ///       зать соответствующее число блоков (это делается при  ини-
    ///       циализации);
    disk_size: u16,
    /// 470 - Номер блока первого файла. Величина также изменяемая;
    start_block: u16,
    raw: [u8; META_SIZE],
}

impl Debug for Meta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meta")
            .field("files", &self.files)
            .field("blocks", &self.blocks)
            .field("microdos_label", &format_args!("{:o}", self.microdos_label))
            .field("mkdos_label", &format_args!("{:o}", self.mkdos_label))
            .field("disk_size", &self.disk_size)
            .field("start_block", &self.start_block)
            // .field("raw", &self.raw)
            .finish()
    }
}

impl Meta {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            files: 0,
            blocks: 0,
            microdos_label: MICRODOS_LABEL,
            mkdos_label: MKDOS_LABEL,
            disk_size: 0,
            start_block: 0,
            raw: [0; META_SIZE],
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum DirEntryStatus {
    /// 0 - обычный;
    Normal = 0,
    /// 1 - защищен;
    Protected = 1,
    /// 2 - логический диск;
    LogicalDisk = 2,
    /// 4 - Дира (такого статуса в mkdos нет!!!);
    Directory = 4,
    /// 200 - BAD-файл;
    BadFile = 0o200,
    /// 377 - удален.
    Deleted = 0o377,
}

impl Default for DirEntryStatus {
    fn default() -> Self {
        Self::Normal
    }
}

impl From<DirEntryStatus> for u8 {
    fn from(des: DirEntryStatus) -> Self {
        des as u8
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(usize)]
pub enum DirEntryOffset {
    /// 0 - Статус файла;
    /// DirEntryStatus
    Status = 0,
    /// 1 - Номер подкаталога (0 - корень);
    DirNo = 1,
    /// 2 - Имя файла 14. символов ASCII KOI8;
    Name = 2,
    /// 20 - Номер блока;
    StartBlock = 0o20,
    /// 22 - Длина в блоках;
    Blocks = 0o22,
    /// 24 - Адрес;
    StartAddress = 0o24,
    /// 26 - Длина.
    Length = 0o26,
}

#[derive(Clone)]
pub struct DirEntry {
    /// 0 - Статус файла;
    /// DirEntryStatus
    pub status: DirEntryStatus,
    /// 1 - Номер подкаталога (0 - корень);
    pub dir_no: u8,
    /// 2 - Имя файла 14. символов ASCII KOI8;
    pub name: String,
    /// 20 - Номер блока;
    pub start_block: u64,
    /// 22 - Длина в блоках;
    pub blocks: u64,
    /// 24 - Адрес;
    pub start_address: u32,
    /// 26 - Длина.
    pub length: u32,
    /// Реальный размер
    pub size: u32,
    /// virtual inode
    /// 1..1000 - direcory inode
    /// 1000.. - other files
    pub inode: u64,
    pub parent_inode: u64,
    pub is_dir: bool,
    pub is_normal: bool,
    // Protected S_ISVTX (01000 - sticky)
    pub is_protected: bool,
    pub is_logical: bool,
    pub is_bad: bool,
    pub is_deleted: bool,
    pub is_unknown: bool,
    /// unix mode
    pub mode: u16,
    raw: [u8; DIR_ENTRY_SIZE],
}

impl Debug for DirEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirEntry")
            .field("status", &self.status)
            .field("dir_no", &self.dir_no)
            .field("name", &self.name)
            .field("start_block", &self.start_block)
            .field("blocks", &self.blocks)
            .field("start_address", &format_args!("{:o}", &self.start_address))
            .field("length", &self.length)
            .field("size", &self.size)
            .field("inode", &self.inode)
            .field("parent_inode", &self.parent_inode)
            .field("is_dir", &self.is_dir)
            .field("is_normal", &self.is_normal)
            .field("is_protected", &self.is_protected)
            .field("is_logical", &self.is_logical)
            .field("is_bad", &self.is_bad)
            .field("is_deleted", &self.is_deleted)
            .field("is_unknown", &self.is_unknown)
            .field("mode", &format_args!("{:o}", &self.mode))
            // .field("raw", &self.raw)
            .finish()
    }
}

impl DirEntry {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for DirEntry {
    fn default() -> Self {
        Self {
            status: DirEntryStatus::default(),
            dir_no: 0,
            name: String::default(),
            start_block: 0,
            blocks: 0,
            start_address: 0,
            length: 0,
            size: 0,
            inode: 0,
            parent_inode: 0,
            is_dir: false,
            is_normal: false,
            is_protected: false,
            is_logical: false,
            is_bad: false,
            is_deleted: false,
            is_unknown: false,
            // r--r--r-- ;)
            mode: 0o0444,
            raw: [0; DIR_ENTRY_SIZE],
        }
    }
}

pub struct Fs {
    /// path to image
    file_path: String,
    /// read only mode
    read_only: bool,
    reader: Option<Reader>,
    #[allow(dead_code)]
    writer: Option<File>,
    offset: u64,
    size: u64,
    inverted: bool,
    last_modified: SystemTime,
    /// image meta block
    meta: Meta,
    /// directory inodes
    dir_inodes: AtomicU64,
    /// file inodes
    file_inodes: AtomicU64,
    /// next free file handle
    next_fh: AtomicU64,
    /// directory entries,
    entries: Vec<DirEntry>,
    _tracing_span: tracing::Span,
}

impl std::fmt::Debug for Fs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fs")
            .field("file_name", &self.file_path)
            .field("read_only", &self.read_only)
            // .field("reader", &self.reader)
            // .field("writer", &self.writer)
            .field("meta", &self.meta)
            .field("offset", &self.offset)
            .field("size", &self.size)
            .field("inverted", &self.inverted)
            .field("dir_inodes", &self.dir_inodes)
            .field("file_inodes", &self.file_inodes)
            .field("next_fh", &self.next_fh)
            .field("entries", &self.entries)
            .finish()
    }
}

impl Default for Fs {
    fn default() -> Self {
        Self {
            file_path: String::default(),
            read_only: true,
            reader: None,
            writer: None,
            offset: 0,
            size: 0,
            inverted: false,
            last_modified: SystemTime::UNIX_EPOCH,
            meta: Meta::new(),
            dir_inodes: AtomicU64::new(2),
            file_inodes: AtomicU64::new(1001),
            next_fh: AtomicU64::new(1),
            entries: Vec::new(),
            _tracing_span: tracing::span!(tracing::Level::TRACE, "Fs"),
        }
    }
}

#[derive(Error, Debug)]
pub enum FsError {
    #[error("Fuser init function error): {0}")]
    FuserInitError(i32),
    #[error("Bad meta size: {0}")]
    BadMetaSize(usize),
    #[error("Can't find MicroDOS label")]
    LabelMicroDos,
    #[error("Can't find MKDOS label")]
    LabelMkDos,
    #[error("Unknown size in image with offset. Must use set_size_blocks()")]
    UnknownSize,
    #[error("Io: {desc}")]
    CustomIo {
        desc: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Io Error")] // #[error(transparent)]
    Io {
        #[from]
        source: std::io::Error,
    },
    #[error("Uknown Error")]
    Unknown,
}

impl Fs {
    pub fn new(fname: &str) -> Self {
        Self {
            file_path: fname.into(),
            ..Default::default()
        }
    }

    #[instrument(level = "trace", skip(self), fields(file_path, ?self.file_path))]
    pub fn try_open(&mut self) -> Result<(), FsError> {
        let fname = PathBuf::new().join(&self.file_path);
        let h = OpenOptions::new()
            .read(true)
            .write(!self.read_only)
            .append(false)
            .open(&fname)
            .map_err(|e| FsError::CustomIo {
                desc: format!("Can't open {:?}", &fname),
                source: e,
            })?;
        if self.size == 0 {
            if self.offset != 0 {
                return Err(FsError::UnknownSize);
            }
            let m = h.metadata()?;
            self.size = m.blocks() * BLOCK_SIZE as u64;
            self.last_modified = m.modified()?;
        }
        let reader = if self.inverted {
            Reader::inverted(h)
        } else {
            Reader::new(h)
        };
        self.reader = Some(reader);
        self.read_meta()?;
        self.read_entries()?;

        // return Err(FsError::Unknown);

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    fn read_meta(&mut self) -> Result<(), FsError> {
        // warn!(parent: &self._tracing_span, "TESTING TARGET: _tracing_span");
        if let Some(reader) = self.reader.as_mut() {
            let buf = &mut self.meta.raw[..];
            // let size = reader.read_at(buf, 0)?;
            let _pos = reader.seek(SeekFrom::Start(self.offset))?;
            let size = reader.read(buf)?;
            if size < META_SIZE {
                return Err(FsError::BadMetaSize(size));
            }
            let mut buf = &self.meta.raw[..];
            buf.advance(MetaOffset::Files as usize);
            self.meta.files = buf.get_u16_le();
            self.meta.blocks = buf.get_u16_le();
            buf.advance(MetaOffset::LabelsOffset as usize);
            let label = buf.get_u16_le();
            if label != MICRODOS_LABEL {
                return Err(FsError::LabelMicroDos);
            }
            let label = buf.get_u16_le();
            if label != MKDOS_LABEL {
                return Err(FsError::LabelMkDos);
            }
            buf.advance(MetaOffset::DiskSizeOffset as usize);
            self.meta.disk_size = buf.get_u16_le();
            self.meta.start_block = buf.get_u16_le();
            if self.meta.start_block < 20 {
                warn!(parent: &self._tracing_span, "Start block record = {} less than 20 Strange!", self.meta.start_block);
            }

            trace!(?self.meta);

            if (self.size / BLOCK_SIZE as u64) < self.meta.disk_size as u64 {
                warn!(parent: &self._tracing_span, "Wrong (corrupted?) disk size {} in meta block but image size is {}", self.meta.disk_size, (self.size / BLOCK_SIZE as u64));
            }
        } else {
            todo!("Need to Reopen");
            // Ok(())
        }

        Ok(())
    }

    #[instrument(level = "trace", skip(self))]
    fn read_entries(&mut self) -> Result<(), FsError> {
        let mut cur_pos = MetaOffset::DirEntriesStart as u64 + self.offset;
        let mut count_all = 0;
        let mut count_normal = 0;
        let mut count_logical = 0;
        let mut count_deleted = 0;
        let mut count_bad = 0;
        let mut used_blocks = 0;
        let mut bad_blocks = 0;
        let mut hole_blocks = 0;
        if let Some(reader) = self.reader.as_mut() {
            let _pos = reader.seek(SeekFrom::Start(cur_pos as u64))?;
            // dbg!(&pos);
            use DirEntryStatus::*;
            loop {
                let mut dentry = DirEntry::new();
                let buf = &mut dentry.raw[..];
                // reader.seek(SeekFrom::Start(cur_pos as u64))?;
                let _ = reader.read(buf)?;
                let mut buf = &dentry.raw[..];

                let f_status = buf.get_u8();
                let dir_no = buf.get_u8();
                let name = buf.get(..14).unwrap();
                if name[0] == 0u8 {
                    break;
                }
                buf.advance(14);
                let start_block = buf.get_u16_le();
                let blocks = buf.get_u16_le();
                let start_address = buf.get_u16_le();
                let length = buf.get_u16_le();

                let is_directory = name[0] == 0o177u8;
                let status = if is_directory {
                    dentry.is_dir = true;
                    // вот тут бля вопрос спорный, я не помню как именно удаляются каталоги
                    // надо смотреть в сырцах mkdos-а
                    // но будем пока считать что 0377 - это все еще удаленный, даже если это
                    // каталог
                    if f_status == 0o377 {
                        count_deleted += 1;
                        Deleted
                    } else {
                        // Just a hint :-D
                        count_normal += 1;
                        Directory
                    }
                } else {
                    match f_status {
                        0 => {
                            dentry.is_normal = true;
                            count_normal += 1;
                            used_blocks += blocks;
                            Normal
                        }
                        1 => {
                            dentry.is_protected = true;
                            count_normal += 1;
                            used_blocks += blocks;
                            Protected
                        }
                        2 => {
                            dentry.is_logical = true;
                            count_normal += 1;
                            count_logical += 1;
                            used_blocks += blocks;
                            LogicalDisk
                        }
                        0o200 => {
                            dentry.is_bad = true;
                            count_bad += 1;
                            bad_blocks += blocks;
                            BadFile
                        }
                        0o377 => {
                            dentry.is_deleted = true;
                            count_deleted += 1;
                            hole_blocks += blocks;
                            Deleted
                        }
                        n => {
                            // если мы в конце каталога, то ловить уже нечего
                            // иначе проверим совсем ли это мусор
                            // или еще есть смысл сохранить эту запись
                            if count_normal >= self.meta.files
                                || start_block <= self.meta.start_block
                                || start_block >= self.meta.disk_size
                                || blocks > self.meta.disk_size - self.meta.blocks
                            {
                                break;
                            }

                            warn!(parent: &self._tracing_span, "Uknown Status: 0{:o}", n);
                            dentry.is_unknown = true;
                            Normal
                        }
                    }
                };
                count_all += 1;

                // уберем из имени дирректории служебный симфол
                let name_off = if is_directory { &name[1..] } else { &name };
                let (cow, _encoding_used, had_errors) = KOI8_R.decode(name_off);
                if had_errors {
                    warn!(parent: &self._tracing_span, "Error while recoding file name {:?}", name_off);
                }

                dentry.status = status;
                dentry.dir_no = dir_no;
                // а номер каталога 0 - это корень? будем считать, что да
                // привязываем его к нашим виртуальным инодам, поэтому + 1
                dentry.parent_inode = 1 + dir_no as u64;
                dentry.name = String::from(cow.trim_end());
                dentry.start_block = start_block as u64;
                dentry.blocks = blocks as u64;
                dentry.start_address = start_address as u32;
                dentry.length = length as u32;
                // вот как здесь лучше с размером пуступить пока не знаю
                // сделаем просто
                // по хорошему, если размер в блоках > 128
                // то размер должен расчитывать как blocks * 512 + length
                // при этом в length должна быть < 512
                // но этого вроде нет, поэтому так
                if blocks > (u16::MAX as usize / BLOCK_SIZE + 1) as u16 {
                    dentry.size = blocks as u32 * BLOCK_SIZE as u32;
                } else {
                    dentry.size = length as u32;
                }

                if is_directory {
                    // dentry.inode = self.dir_inodes.fetch_add(1, Ordering::SeqCst)
                    // Изврат, МКТ в курсе :-D
                    dentry.inode = 1 + f_status as u64;
                    dentry.mode = 0o755;
                } else {
                    dentry.inode = self.file_inodes.fetch_add(1, Ordering::SeqCst)
                }
                if dentry.is_protected {
                    dentry.mode |= 0o1000;
                }
                // удаленные и файлы и bad-блоки в dir_no получает 255?
                // получается, что он по любому не попадает при поиске через
                // entries_by_parent_inode, но мы вседа это можем подсунуть вот здесь ;)
                // скажем плюхнуть в корень :)
                if dentry.is_deleted || dentry.is_bad {
                    // dbg!(&dentry);
                    dentry.parent_inode = 1;
                }
                if dentry.is_unknown {
                    warn!(parent: &self._tracing_span,
                          "File with unknown status {:?}", dentry);
                }
                self.entries.push(dentry);

                cur_pos += DIR_ENTRY_SIZE as u64;
                if cur_pos > start_block as u64 * BLOCK_SIZE as u64 + self.offset {
                    break;
                }
            }
        } else {
            todo!("Need to Reopen");
            // Ok(())
        }
        // trace!(parent: &self._tracing_span, "ENTRIES: {:#?}", self.entries);
        debug!(parent: &self._tracing_span,
            count_normal, count_deleted, count_logical, count_bad, count_all, "ENTRIES:"
        );
        // assert_eq!(self.meta.files, count_normal);
        if count_normal != self.meta.files {
            warn!(parent: &self._tracing_span,
                  "Wrong files count? Meta file count is {} but {} found",
                  self.meta.files, count_normal
            );
        }
        debug!(parent: &self._tracing_span,
               used_blocks, bad_blocks, hole_blocks, "ENTRIES:"
        );
        // assert_eq!(self.meta.blocks, used_blocks);
        if used_blocks + self.meta.start_block != self.meta.blocks {
            warn!(parent: &self._tracing_span,
                  "Wrong used blocks? Meta file blocks is {} but {} found",
                  self.meta.blocks, used_blocks + self.meta.start_block
            );
        }

        Ok(())
    }

    pub fn try_reopen(&mut self) -> Result<(), FsError> {
        self.dir_inodes = AtomicU64::new(2);
        self.file_inodes = AtomicU64::new(1001);
        self.size = 0;
        self.meta = Meta::new();
        self.entries = Vec::new();
        // TODO: закрыть все открытые файлы
        // но потом надо будет сделать умное закрытие
        self.try_open()
    }

    pub fn last_modified(&self) -> SystemTime {
        self.last_modified.clone()
    }

    pub fn check_modified(&mut self) -> bool {
        let modified = if let Some(reader) = self.reader.as_ref() {
            let inner = reader.as_ref();
            if let Ok(m) = inner.metadata() {
                match m.modified() {
                    Ok(mt) => {
                        if mt != self.last_modified {
                            warn!(parent: &self._tracing_span, "Disk modified {:?} -> {:?}", self.last_modified, mt);
                            self.last_modified = mt;
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                }
            } else {
                false
            }
        } else {
            todo!()
        };
        if modified {
            match self.try_reopen() {
                Ok(_) => {
                    warn!(parent: &self._tracing_span, "Try to reopen");
                    true
                }
                // TODO: закрываем все нахрен и вываливаемся с ошибкой
                Err(_) => panic!("Can't reopen"),
            }
        } else {
            false
        }
    }

    pub fn entries_by_parent_inode(&mut self, parent_ino: u64) -> Vec<DirEntry> {
        let _ = self.check_modified();
        self.entries
            .iter()
            .filter(|&entry| entry.parent_inode == parent_ino)
            // .map(|x| x.clone())
            .cloned()
            .collect()
    }

    /// ищет `name` в фолдере с `parent_inode`
    /// понятно что для mkdos это нафиг не надо ибо подкаталоги
    /// просто для красоты, но если будем маскировать логические диски
    /// под каталоги, то надо будет именно так
    pub fn find_entrie(&mut self, name: &str, parent_inode: u64) -> Option<&DirEntry> {
        let _ = self.check_modified();
        self.entries
            .iter()
            .find(|&entry| entry.parent_inode == parent_inode && entry.name == name)
    }

    pub fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> Result<usize, std::io::Error> {
        if let Some(reader) = self.reader.as_mut() {
            let _pos = reader.seek(SeekFrom::Start(self.offset + offset))?;
            reader.read(buf)
        } else {
            todo!()
        }
    }

    pub fn entrie_by_inode(&mut self, inode: u64) -> Option<&DirEntry> {
        let _ = self.check_modified();
        self.entries.iter().find(|&entry| entry.inode == inode)
    }

    pub fn block_size(&self) -> u64 {
        BLOCK_SIZE as u64
    }

    pub fn files(&self) -> u64 {
        self.meta.files as u64
    }

    pub fn disk_size(&self) -> u64 {
        self.meta.disk_size as u64
    }

    pub fn blocks(&self) -> u64 {
        self.meta.blocks as u64
    }

    /// Set the fs's offset.
    pub fn set_offset(&mut self, offset: u64) {
        self.offset = offset;
    }

    /// Set the fs's offset in blocks.
    pub fn set_offset_blocks(&mut self, offset: u64) {
        self.offset = offset * BLOCK_SIZE as u64;
    }

    /// Set the fs's size
    pub fn set_size(&mut self, size: u64) {
        self.size = size;
    }

    /// Set the fs's size in blocks.
    pub fn set_size_blocks(&mut self, size: u64) {
        self.size = size * BLOCK_SIZE as u64;
    }

    /// Set the fs's inverted.
    pub fn set_inverted(&mut self, inverted: bool) {
        self.inverted = inverted;
    }
}
