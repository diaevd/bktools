use libc::{ENOENT, ENOSYS};
use std::{
    ffi::OsStr,
    time::{Duration as StdDuration, SystemTime as StdSystemTime, UNIX_EPOCH as STD_UNIX_EPOCH},
};
use time::macros::datetime;

use fuser::{
    FileType, Filesystem, KernelConfig, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyDirectoryPlus, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyLock, ReplyLseek,
    ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use mkdosfs::{DirEntryStatus, Fs, FsError};

use tracing::instrument;

const ED_UNIX_TIME: u64 = 286405200;

fn from_direntry_status(status: DirEntryStatus) -> FileType {
    use DirEntryStatus::*;

    match status {
        Normal | Protected | LogicalDisk => FileType::RegularFile,
        Directory => FileType::Directory,
        BadFile => FileType::RegularFile,
        Deleted => FileType::RegularFile,
    }
}

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
    kind: FileType::Directory,
    perm: 0o755,
    nlink: 2,
    uid: 1000,
    gid: 1000,
    rdev: 0,
    flags: 0,
    blksize: 512,
};

#[derive(Debug)]
pub struct FuseFs {
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
    ///
    fs: Fs,
    _tracing_span: tracing::Span,
}

impl Default for FuseFs {
    fn default() -> Self {
        Self {
            file_path: String::default(),
            _tracing_span: tracing::span!(tracing::Level::TRACE, "FuseFs"),
            demonize: false,
            read_only: true,
            show_bad: false,
            show_deleted: false,
            fs: Fs::default(),
        }
    }
}

impl FuseFs {
    pub fn new(fname: &str) -> Result<Self, FsError> {
        let mut fs = Fs::new(fname);
        fs.try_open()?;
        Ok(Self {
            fs,
            ..Default::default()
        })
    }

    pub fn show_bad(&mut self, arg: bool) {
        self.show_bad = arg;
    }

    pub fn show_deleted(&mut self, arg: bool) {
        self.show_deleted = arg;
    }
}

impl Filesystem for FuseFs {
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
        if let Some(entry) = self.fs.find_entrie(name.to_str().unwrap(), parent) {
            let fattr = FileAttr {
                ino: entry.inode,
                size: entry.length as u64,
                blocks: entry.blocks,
                atime: datetime!(1979-01-29 03:00 UTC).into(),
                mtime: datetime!(1979-01-29 03:00 UTC).into(),
                ctime: datetime!(1979-01-29 03:00 UTC).into(),
                crtime: datetime!(1979-01-29 03:00 UTC).into(),
                kind: from_direntry_status(entry.status),
                perm: entry.mode,
                nlink: 1,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                blksize: self.fs.block_size() as u32,
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
            reply.attr(&StdDuration::from_secs(10), &dattr);
        }
        // 2 => _
        else if let Some(entry) = self.fs.entrie_by_inode(ino) {
            let fattr = FileAttr {
                ino,
                size: entry.length as u64,
                blocks: entry.blocks,
                atime: datetime!(1979-01-29 03:00 UTC).into(),
                mtime: datetime!(1979-01-29 03:00 UTC).into(),
                ctime: datetime!(1979-01-29 03:00 UTC).into(),
                crtime: datetime!(1979-01-29 03:00 UTC).into(),
                kind: from_direntry_status(entry.status),
                perm: entry.mode,
                nlink: 1,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                blksize: self.fs.block_size() as u32,
                flags: 0,
            };
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

        if let Some(entry) = self.fs.entrie_by_inode(ino) {
            let file_size = entry.length as u64;
            // Could underflow if file length is less than local_start
            let read_size = std::cmp::min(size, file_size.saturating_sub(offset as u64) as u32);
            // Move this to mkfdosfs::Fs
            let real_offset = offset as u64 + entry.start_block * self.fs.block_size();
            let mut buf = vec![0; read_size as usize];
            // ^
            if self.fs.read_exact_at(&mut buf, real_offset).is_ok() {
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
                    if reply.add(1, offset, FileType::Directory, ".") {
                        return;
                    }
                }
                if offset == 1 {
                    offset += 1;
                    if reply.add(1, offset, FileType::Directory, "..") {
                        return;
                    }
                }
            } else {
                if offset == 0 {
                    offset += 1;
                    if reply.add(ino, offset, FileType::Directory, ".") {
                        return;
                    }
                }
                if offset == 1 {
                    let entry = self.fs.entrie_by_inode(ino);
                    assert!(entry.is_some());
                    offset += 1;
                    if reply.add(
                        entry.unwrap().parent_inode,
                        offset,
                        FileType::Directory,
                        "..",
                    ) {
                        return;
                    }
                }
            }
        }

        // фильтр надо перести в mkdosfs
        for (i, entry) in self
            .fs
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
                from_direntry_status(entry.status),
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
        // let _ffree = (self.meta.start_block as u64 * BLOCK_SIZE as u64
        //     - MetaOffset::DirEntriesStart as u64)
        //     / DIR_ENTRY_SIZE as u64
        //     - self.meta.files as u64;
        // dbg!(_ffree);
        reply.statfs(
            self.fs.disk_size(),
            self.fs.disk_size() - self.fs.blocks(),
            self.fs.disk_size() - self.fs.blocks() as u64,
            self.fs.files(),
            0,
            self.fs.block_size() as u32,
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
