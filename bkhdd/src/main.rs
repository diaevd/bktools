use binrw::BinRead;
use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
use std::{
    fs,
    io::{Cursor, Read, Seek, SeekFrom, Write},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

use bkhdd::{
    io::{BinInvertedReader, ReverseReader},
    AHDDLayout, HDILayout, AHDD, AHDD_CYL_B, AHDD_LOGD_B, AHDD_PT_SEC, BLOCK_SIZE,
};

fn main() -> Result<()> {
    setup_logging()?;

    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .setting(AppSettings::ColoredHelp)
        // .subcommand(subcmd)
        .arg(
            Arg::with_name("IMAGE_NAME")
                .required(true)
                .index(1)
                .help("MKDOS disk image file path"),
        )
        .get_matches();

    info!("Starting");

    let image_name = matches.value_of("IMAGE_NAME").unwrap();

    let mut f = fs::File::open(image_name)?;
    f.seek(SeekFrom::Start(0))?;
    let hdi = HDILayout::read(&mut f)?;
    dbg!(&hdi);

    let mut ahdd = AHDD::new(image_name);
    ahdd.set_offset(BLOCK_SIZE as u64);
    ahdd.read_header()?;

    let part = &ahdd.partitions()[0];
    let pos = part.lba;
    dbg!(&pos);

    let offset = BLOCK_SIZE as u64 + (pos as u64 * BLOCK_SIZE as u64);
    let pos = f.seek(SeekFrom::Start(offset))?;
    dbg!(&pos);

    let mut buf = [0u8; BLOCK_SIZE];
    let mut reader = BinInvertedReader::new(f);
    let size = reader.read(&mut buf)?;
    assert_eq!(size, BLOCK_SIZE);
    let mut w = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .append(false)
        .open("test_block.dump")?;
    w.write(&buf[..])?;
    w.flush()?;

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
