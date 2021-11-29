//#![feature(destructuring_assignment)]

use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
use fuser::MountOption;
use tracing::info;
use tracing_subscriber::EnvFilter;

use fuse_mkdosfs::FuseFs;

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
    let mut fs = FuseFs::new(imagename)?;

    if matches.is_present("show-bad") {
        fs.show_bad(true);
    }
    if matches.is_present("show-deleted") {
        fs.show_deleted(true);
    }

    info!("Starting");
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
