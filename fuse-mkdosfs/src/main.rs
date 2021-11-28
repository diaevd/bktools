//#![feature(destructuring_assignment)]

use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
use fuser::MountOption;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::mkdosfs::Fs;

pub mod mkdosfs {
    use std::{
        ffi::OsStr,
        fmt::Debug,
        fs::{File, OpenOptions},
        io::{Seek, SeekFrom},
        os::unix::fs::{FileExt, MetadataExt},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use ::time::macros::datetime;
    use bytes::Buf;
    use encoding_rs::KOI8_R;
    use fuser::{
        Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory,
        ReplyDirectoryPlus, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek, ReplyOpen,
        ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
    };
    use libc::{ENOENT, ENOSYS};
    use thiserror::Error;
    use tracing::{debug, instrument, trace, warn};

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

    impl From<DirEntryStatus> for fuser::FileType {
        fn from(des: DirEntryStatus) -> Self {
            match des {
                DirEntryStatus::Normal => fuser::FileType::RegularFile,
                DirEntryStatus::Protected => fuser::FileType::RegularFile,
                DirEntryStatus::LogicalDisk => fuser::FileType::RegularFile,
                DirEntryStatus::Directory => fuser::FileType::Directory,
                DirEntryStatus::BadFile => fuser::FileType::RegularFile,
                DirEntryStatus::Deleted => fuser::FileType::RegularFile,
            }
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
        status: DirEntryStatus,
        /// 1 - Номер подкаталога (0 - корень);
        dir_no: u8,
        /// 2 - Имя файла 14. символов ASCII KOI8;
        name: String,
        /// 20 - Номер блока;
        start_block: u64,
        /// 22 - Длина в блоках;
        blocks: u64,
        /// 24 - Адрес;
        start_address: u32,
        /// 26 - Длина.
        length: u32,
        /// virtual inode
        /// 1..1000 - direcory inode
        /// 1000.. - other files
        inode: u64,
        parent_inode: u64,
        is_dir: bool,
        is_normal: bool,
        /// Protected S_ISVTX (01000 - sticky)
        is_protected: bool,
        is_logical: bool,
        is_bad: bool,
        is_deleted: bool,
        /// unix mode
        mode: u16,
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
                .field("inode", &self.inode)
                .field("parent_inode", &self.parent_inode)
                .field("is_dir", &self.is_dir)
                .field("is_normal", &self.is_normal)
                .field("is_protected", &self.is_protected)
                .field("is_logical", &self.is_logical)
                .field("is_bad", &self.is_bad)
                .field("is_deleted", &self.is_deleted)
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
                inode: 0,
                parent_inode: 0,
                is_dir: false,
                is_normal: false,
                is_protected: false,
                is_logical: false,
                is_bad: false,
                is_deleted: false,
                // r--r--r-- ;)
                mode: 0o0444,
                raw: [0; DIR_ENTRY_SIZE],
            }
        }
    }

    pub struct Fs {
        /// path to image
        file_path: String,
        /// need demonize
        demonize: bool,
        /// read only mode
        read_only: bool,
        /// Enable show bad files
        show_bad: bool,
        /// Enable show deleted
        show_deleted: bool,
        reader: Option<File>,
        writer: Option<File>,
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
                .field("demonize", &self.demonize)
                .field("read_only", &self.read_only)
                .field("show_bad", &self.show_bad)
                .field("show_deleted", &self.show_deleted)
                .field("reader", &self.reader)
                .field("writer", &self.writer)
                .field("meta", &self.meta)
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
                demonize: false,
                read_only: true,
                show_bad: false,
                show_deleted: false,
                reader: None,
                writer: None,
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
        #[error("Can't find NicroDOS label")]
        LabelMicroDos,
        #[error("Can't find MKDOS label")]
        LabelMkDos,
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
            // trace!("TEST1");
            let _ = tracing::Span::current().enter();
            // trace!("TEST2");

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
            self.reader = Some(h);
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
                // reader.seek(io::SeekFrom::Start(0))?;
                // reader.read(buf)?;
                let size = reader.read_at(buf, 0)?;
                if size < META_SIZE {
                    return Err(FsError::BadMetaSize(size));
                }

                let mut buf = &self.meta.raw[..];
                // let mut meta = Meta::new();
                buf.advance(MetaOffset::Files as usize);
                self.meta.files = buf.get_u16_le();
                self.meta.blocks = buf.get_u16_le();
                // buf.advance(
                //     usize::from(MetaOffset::MicrodosLabel)
                //         - usize::from(MetaOffset::Blocks)
                //         - 2 as usize,
                // );
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

                trace!(?self.meta);

                let meta = reader.metadata()?;
                trace!(?meta);

                if meta.blocks() < self.meta.disk_size as u64 {
                    warn!(parent: &self._tracing_span, "Wrong (corrupted?) disk size {} in meta block but image size is {}", self.meta.disk_size, meta.blocks());
                }
            } else {
                todo!("Need to Reopen");
                // Ok(())
            }

            Ok(())
        }

        #[instrument(level = "trace", skip(self))]
        fn read_entries(&mut self) -> Result<(), FsError> {
            let mut cur_pos = MetaOffset::DirEntriesStart as u64;
            let mut count_all = 0;
            let mut count_normal = 0;
            let mut count_deleted = 0;
            let mut count_bad = 0;
            let mut used_blocks = 0;
            let mut bad_blocks = 0;
            let mut hole_blocks = 0;
            if let Some(reader) = self.reader.as_mut() {
                reader.seek(SeekFrom::Start(cur_pos as u64))?;
                use DirEntryStatus::*;
                loop {
                    let mut dentry = DirEntry::new();
                    let buf = &mut dentry.raw[..];
                    reader.read_at(buf, cur_pos)?;

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
                                warn!(parent: &self._tracing_span, "Uknown Status: 0{:o}", n);
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
                    self.entries.push(dentry);

                    cur_pos += DIR_ENTRY_SIZE as u64;
                    if cur_pos > start_block as u64 * BLOCK_SIZE as u64 {
                        break;
                    }
                }
            } else {
                todo!("Need to Reopen");
                // Ok(())
            }
            // trace!(parent: &self._tracing_span, "ENTRIES: {:#?}", self.entries);
            debug!(parent: &self._tracing_span,
                count_normal, count_deleted, count_all, count_bad, "ENTRIES:"
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
            if used_blocks != self.meta.blocks {
                warn!(parent: &self._tracing_span,
                      "Wrong used blocks? Meta file blocks is {} but {} found",
                      self.meta.blocks, used_blocks
                );
            }

            Ok(())
        }

        fn entries_by_parent_inode(&self, parent_ino: u64) -> Vec<DirEntry> {
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
        fn find_entrie(&self, name: &str, parent_inode: u64) -> Option<&DirEntry> {
            self.entries
                .iter()
                .find(|&entry| entry.parent_inode == parent_inode && entry.name == name)
        }

        fn entrie_by_inode(&self, inode: u64) -> Option<&DirEntry> {
            self.entries.iter().find(|&entry| entry.inode == inode)
        }

        /// Set the fs's is demonize.
        pub fn with_demonize(&mut self, demonize: bool) {
            self.demonize = demonize;
        }

        /// Set the fs's read only.
        pub fn set_read_only(&mut self, read_only: bool) {
            self.read_only = read_only;
        }

        pub fn show_bad(&mut self, arg: bool) {
            self.show_bad = arg;
        }

        pub fn show_deleted(&mut self, arg: bool) {
            self.show_deleted = arg;
        }
    }

    // use std::time::SystemTime;
    use std::time::{
        Duration as StdDuration, SystemTime as StdSystemTime, UNIX_EPOCH as STD_UNIX_EPOCH,
    };

    const ED_UNIX_TIME: u64 = 286405200;

    fn systime_from_secs(secs: u64) -> StdSystemTime {
        STD_UNIX_EPOCH + StdDuration::from_secs(secs)
    }

    const ROOT_DIR_ATTR: fuser::FileAttr = fuser::FileAttr {
        ino: 1,
        size: 0,
        blocks: 0,
        atime: STD_UNIX_EPOCH, // 1970-01-01 00:00:00
        mtime: STD_UNIX_EPOCH,
        ctime: STD_UNIX_EPOCH,
        crtime: STD_UNIX_EPOCH,
        kind: fuser::FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 512,
    };

    impl Filesystem for Fs {
        #[instrument(level = "trace")]
        fn init(
            &mut self,
            _req: &Request<'_>,
            _config: &mut KernelConfig,
        ) -> std::result::Result<(), i32> {
            Ok(())
        }

        fn destroy(&mut self) {}

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            use fuser::FileAttr;

            // dbg!("LOOKUP: ", parent, name);
            if let Some(entry) = self.find_entrie(name.to_str().unwrap(), parent) {
                let fattr = FileAttr {
                    ino: entry.inode,
                    size: entry.length as u64,
                    blocks: entry.blocks,
                    atime: datetime!(1979-01-29 03:00 UTC).into(),
                    mtime: datetime!(1979-01-29 03:00 UTC).into(),
                    ctime: datetime!(1979-01-29 03:00 UTC).into(),
                    crtime: datetime!(1979-01-29 03:00 UTC).into(),
                    kind: entry.status.into(),
                    perm: entry.mode,
                    nlink: 1,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    blksize: BLOCK_SIZE as u32,
                    flags: 0,
                };
                reply.entry(&StdDuration::from_secs(10), &fattr, 0);
            } else {
                reply.error(ENOENT);
            }
        }

        fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
            use fuser::FileAttr;
            // 1 => _
            if ino == 1 {
                let mut dattr = ROOT_DIR_ATTR;
                dattr.atime = datetime!(1979-01-29 03:00 UTC).into(); //systime_from_secs(ED_UNIX_TIME);
                dattr.ctime = systime_from_secs(ED_UNIX_TIME);
                dattr.mtime = systime_from_secs(ED_UNIX_TIME);
                dattr.crtime = systime_from_secs(ED_UNIX_TIME);
                //dbg!("getattr", ino, &dattr);
                reply.attr(&StdDuration::from_secs(10), &dattr);
            }
            // 2 => _
            else if let Some(entry) = self.entrie_by_inode(ino) {
                let fattr = FileAttr {
                    ino,
                    size: entry.length as u64,
                    blocks: entry.blocks,
                    atime: datetime!(1979-01-29 03:00 UTC).into(),
                    mtime: datetime!(1979-01-29 03:00 UTC).into(),
                    ctime: datetime!(1979-01-29 03:00 UTC).into(),
                    crtime: datetime!(1979-01-29 03:00 UTC).into(),
                    kind: entry.status.into(),
                    perm: entry.mode,
                    nlink: 1,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    blksize: BLOCK_SIZE as u32,
                    flags: 0,
                };
                //dbg!("getattr", ino, &fattr);
                reply.attr(&StdDuration::from_secs(10), &fattr)
            } else {
                reply.error(ENOENT);
            }
            // reply.error(ENOENT);
        }

        fn setattr(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _mode: Option<u32>,
            _uid: Option<u32>,
            _gid: Option<u32>,
            _size: Option<u64>,
            _atime: Option<TimeOrNow>,
            _mtime: Option<TimeOrNow>,
            _ctime: Option<StdSystemTime>,
            _fh: Option<u64>,
            _crtime: Option<StdSystemTime>,
            _chgtime: Option<StdSystemTime>,
            _bkuptime: Option<StdSystemTime>,
            _flags: Option<u32>,
            reply: ReplyAttr,
        ) {
            reply.error(ENOSYS);
        }

        fn readlink(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyData) {
            reply.error(ENOSYS);
        }

        fn mknod(
            &mut self,
            _req: &Request<'_>,
            _parent: u64,
            _name: &OsStr,
            _mode: u32,
            _umask: u32,
            _rdev: u32,
            reply: ReplyEntry,
        ) {
            reply.error(ENOSYS);
        }

        fn mkdir(
            &mut self,
            _req: &Request<'_>,
            _parent: u64,
            _name: &OsStr,
            _mode: u32,
            _umask: u32,
            reply: ReplyEntry,
        ) {
            reply.error(ENOSYS);
        }

        fn unlink(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
            reply.error(ENOSYS);
        }

        fn rmdir(&mut self, _req: &Request<'_>, _parent: u64, _name: &OsStr, reply: ReplyEmpty) {
            reply.error(ENOSYS);
        }

        fn symlink(
            &mut self,
            _req: &Request<'_>,
            _parent: u64,
            _name: &OsStr,
            _link: &std::path::Path,
            reply: ReplyEntry,
        ) {
            reply.error(ENOSYS);
        }

        fn rename(
            &mut self,
            _req: &Request<'_>,
            _parent: u64,
            _name: &OsStr,
            _newparent: u64,
            _newname: &OsStr,
            _flags: u32,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        fn link(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _newparent: u64,
            _newname: &OsStr,
            reply: ReplyEntry,
        ) {
            reply.error(ENOSYS);
        }

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
            // dbg!(ino, flags);
            let access_mask = match flags & libc::O_ACCMODE {
                libc::O_RDONLY => {
                    // Behavior is undefined, but most filesystems return EACCES
                    if flags & libc::O_TRUNC != 0 {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::R_OK
                }
                libc::O_WRONLY => {
                    if self.read_only {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::W_OK
                }
                libc::O_RDWR => {
                    if self.read_only {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::R_OK | libc::W_OK
                }
                // Exactly one access mode flag must be specified
                _ => {
                    reply.error(libc::EINVAL);
                    return;
                }
            };

            reply.opened(0, access_mask as u32);
            // reply.opened(0, 0);
        }

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn read(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            size: u32,
            flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyData,
        ) {
            // dbg!(ino, fh, offset, size, flags);

            if let Some(entry) = self.entrie_by_inode(ino) {
                let file_size = entry.length as u64;
                // Could underflow if file length is less than local_start
                let read_size = std::cmp::min(size, file_size.saturating_sub(offset as u64) as u32);
                let real_offset = offset as u64 + entry.start_block * BLOCK_SIZE as u64;
                if let Some(reader) = self.reader.as_mut() {
                    let mut buf = vec![0; read_size as usize];
                    // fix unwrap
                    reader.read_exact_at(&mut buf, real_offset).unwrap();
                    reply.data(&buf);
                } else {
                    reply.error(libc::EIO);
                }
            } else {
                reply.error(ENOENT);
            }
        }

        fn write(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _offset: i64,
            _data: &[u8],
            _write_flags: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyWrite,
        ) {
            reply.error(ENOSYS);
        }

        fn flush(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _lock_owner: u64,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        fn release(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _flags: i32,
            _lock_owner: Option<u64>,
            _flush: bool,
            reply: ReplyEmpty,
        ) {
            reply.ok();
        }

        fn fsync(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _datasync: bool,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn opendir(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
            let access_mask = match flags & libc::O_ACCMODE {
                libc::O_RDONLY => {
                    // Behavior is undefined, but most filesystems return EACCES
                    if flags & libc::O_TRUNC != 0 {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::R_OK
                }
                libc::O_WRONLY => {
                    if self.read_only {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::W_OK
                }
                libc::O_RDWR => {
                    if self.read_only {
                        reply.error(libc::EACCES);
                        return;
                    }
                    libc::R_OK | libc::W_OK
                }
                // Exactly one access mode flag must be specified
                _ => {
                    reply.error(libc::EINVAL);
                    return;
                }
            };

            reply.opened(0, access_mask as u32);
            // reply.opened(0, 0);
        }

        #[instrument(level = "trace", skip(self, _req, reply))]
        fn readdir(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            mut offset: i64,
            mut reply: ReplyDirectory,
        ) {
            // dbg!("Readdir", ino, offset);
            // эту порнуху надо исправить
            if offset == 0 || offset == 1 {
                if ino == 1 {
                    if offset == 0 {
                        offset += 1;
                        if reply.add(1, offset, fuser::FileType::Directory, ".") {
                            return;
                        }
                    }
                    if offset == 1 {
                        offset += 1;
                        if reply.add(1, offset, fuser::FileType::Directory, "..") {
                            return;
                        }
                    }
                } else {
                    if offset == 0 {
                        offset += 1;
                        if reply.add(ino, offset, fuser::FileType::Directory, ".") {
                            return;
                        }
                    }
                    if offset == 1 {
                        let entry = self.entrie_by_inode(ino);
                        assert!(entry.is_some());
                        offset += 1;
                        if reply.add(
                            entry.unwrap().parent_inode,
                            offset,
                            fuser::FileType::Directory,
                            "..",
                        ) {
                            return;
                        }
                    }
                }
            }

            // фильтр надо перести в mkdosfs
            for (i, entry) in self
                .entries_by_parent_inode(ino)
                .iter()
                .filter(|&e|
                        // !(!self.show_deleted && e.is_deleted) && !(!self.show_bad && e.is_bad))
                        (!e.is_deleted || self.show_deleted) && (!e.is_bad || self.show_bad))
                .skip((offset - 2) as usize)
                .enumerate()
            {
                if reply.add(
                    entry.inode,
                    // i + 1 means the index of the next entry
                    offset + 1 + i as i64,
                    entry.status.into(),
                    &entry.name,
                ) {
                    break;
                }
            }

            reply.ok();
            // reply.error(ENOSYS);
        }

        fn readdirplus(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _offset: i64,
            reply: ReplyDirectoryPlus,
        ) {
            reply.error(ENOSYS);
        }

        fn releasedir(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _flags: i32,
            reply: ReplyEmpty,
        ) {
            reply.ok();
        }

        fn fsyncdir(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _datasync: bool,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        /// Returns image fs info
        #[instrument(level = "trace", skip(self, _req, reply))]
        fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
            let _ffree = (self.meta.start_block as u64 * BLOCK_SIZE as u64
                - MetaOffset::DirEntriesStart as u64)
                / DIR_ENTRY_SIZE as u64
                - self.meta.files as u64;
            // dbg!(_ffree);
            reply.statfs(
                self.meta.disk_size as u64,
                (self.meta.disk_size - self.meta.blocks) as u64,
                (self.meta.disk_size - self.meta.blocks) as u64,
                self.meta.files as u64,
                0,
                BLOCK_SIZE as u32,
                14,
                0,
            );
        }

        fn setxattr(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _name: &OsStr,
            _value: &[u8],
            _flags: i32,
            _position: u32,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        fn getxattr(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _name: &OsStr,
            _size: u32,
            reply: ReplyXattr,
        ) {
            reply.error(ENOSYS);
        }

        fn listxattr(&mut self, _req: &Request<'_>, _ino: u64, _size: u32, reply: ReplyXattr) {
            reply.error(ENOSYS);
        }

        fn removexattr(&mut self, _req: &Request<'_>, _ino: u64, _name: &OsStr, reply: ReplyEmpty) {
            reply.error(ENOSYS);
        }

        fn access(&mut self, _req: &Request<'_>, _ino: u64, _mask: i32, reply: ReplyEmpty) {
            reply.error(ENOSYS);
        }

        fn create(
            &mut self,
            _req: &Request<'_>,
            _parent: u64,
            _name: &OsStr,
            _mode: u32,
            _umask: u32,
            _flags: i32,
            reply: ReplyCreate,
        ) {
            reply.error(ENOSYS);
        }

        fn getlk(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _lock_owner: u64,
            _start: u64,
            _end: u64,
            _typ: i32,
            _pid: u32,
            reply: ReplyLock,
        ) {
            reply.error(ENOSYS);
        }

        fn setlk(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _lock_owner: u64,
            _start: u64,
            _end: u64,
            _typ: i32,
            _pid: u32,
            _sleep: bool,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        fn bmap(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _blocksize: u32,
            _idx: u64,
            reply: ReplyBmap,
        ) {
            reply.error(ENOSYS);
        }

        fn ioctl(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _flags: u32,
            _cmd: u32,
            _in_data: &[u8],
            _out_size: u32,
            reply: ReplyIoctl,
        ) {
            reply.error(ENOSYS);
        }

        fn fallocate(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _offset: i64,
            _length: i64,
            _mode: i32,
            reply: ReplyEmpty,
        ) {
            reply.error(ENOSYS);
        }

        fn lseek(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            _fh: u64,
            _offset: i64,
            _whence: i32,
            reply: ReplyLseek,
        ) {
            reply.error(ENOSYS);
        }

        fn copy_file_range(
            &mut self,
            _req: &Request<'_>,
            _ino_in: u64,
            _fh_in: u64,
            _offset_in: i64,
            _ino_out: u64,
            _fh_out: u64,
            _offset_out: i64,
            _len: u64,
            _flags: u32,
            reply: ReplyWrite,
        ) {
            reply.error(ENOSYS);
        }
    }
}

fn main() -> Result<()> {
    setup_logging()?;

    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .setting(AppSettings::ColoredHelp)
        .arg(
            Arg::with_name("IMAGE_NAME")
                .required(true)
                .index(1)
                .help("MKDOS disk image file path"),
        )
        .arg(
            Arg::with_name("MOUNT_POINT")
                .required(true)
                .index(2)
                .help("Mount image at given path"),
        )
        .arg(
            Arg::with_name("auto-unmount")
                .long("auto-unmount")
                .help("Automatically unmount on process exit"),
        )
        .arg(
            Arg::with_name("allow-root")
                .long("allow-root")
                .help("Allow root user to access filesystem"),
        )
        .arg(
            Arg::with_name("show-bad")
                .long("show-bad")
                .help("Enable show bad files (areas marked as bad blocks)"),
        )
        .arg(
            Arg::with_name("show-deleted")
                .long("show-deleted")
                .help("Enable show deleted files (files marked as deleted)"),
        )
        .get_matches();

    let imagename = matches.value_of("IMAGE_NAME").unwrap();
    let mountpoint = matches.value_of("MOUNT_POINT").unwrap();
    let mut options = vec![MountOption::RO, MountOption::FSName("mkdosfs".to_string())];
    if matches.is_present("auto-unmount") {
        options.push(MountOption::AutoUnmount);
    }
    if matches.is_present("allow-root") {
        options.push(MountOption::AllowRoot);
    }

    // fuser::mount2(Fs, mountpoint, &options).wrap_err("fuser::mount error")?;
    info!(?options, "Mount options: ");
    let mut fs = Fs::new(imagename);
    if matches.is_present("show-bad") {
        fs.show_bad(true);
    }
    if matches.is_present("show-deleted") {
        fs.show_deleted(true);
    }

    info!("Starting");

    fs.try_open()?;
    fuser::mount2(fs, mountpoint, &options).map_or_else(
        |e| match e.raw_os_error() {
            Some(0) => Ok(()),
            _ => Err(e),
        },
        Ok,
    )?;

    Ok(())
}

pub fn setup_logging() -> Result<()> {
    if std::env::var("RUST_LIB_BACKTRACE").is_err() {
        std::env::set_var("RUST_LIB_BACKTRACE", "full");
    }
    color_eyre::install()?;

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    Ok(())
}

#[cfg(test)]
mod tests {
    // use std::mem::size_of;

    // use crate::mkdosfs::*;

    // #[test]
    // fn size_of_meta() {
    //     assert_eq!(META_SIZE, size_of::<MetaPacked>());
    // }

    // #[test]
    // fn size_of_meta_dir_entry() {
    //     assert_eq!(DIR_ENTRY_SIZE, size_of::<DirEntryPacked>());
    // }

    // #[test]
    // fn size_of_file_status() {
    //     assert_eq!(1, size_of::<FileStatus>());
    // }
}
