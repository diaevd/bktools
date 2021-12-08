use binrw::BinRead;
use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
use std::{
    fs,
    io::{Cursor, Read, Seek, SeekFrom},
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

    union buffer {
        bytes: [u8; BLOCK_SIZE],
        words: [u16; BLOCK_SIZE / 2],
    }

    let mut buf = buffer {
        bytes: [0u8; BLOCK_SIZE],
    };
    let size_of_buffer = std::mem::size_of::<buffer>();
    assert_eq!(BLOCK_SIZE, size_of_buffer);

    f.seek(SeekFrom::Start(BLOCK_SIZE as u64))?;
    let mut rr = ReverseReader::new(f);
    let size = rr.read(unsafe { &mut buf.bytes[..] })?;
    dbg!(size);
    eprintln!("bytes: {:x?}", unsafe { buf.bytes });
    eprintln!("words: {:x?}", unsafe { buf.words });

    let mut ahdd = AHDD::new(image_name);
    ahdd.set_offset(BLOCK_SIZE as u64);
    ahdd.read_header()?;
    // let mut c = Cursor::new(&buf);

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
