use std::ops::{self, Neg};

use binrw::BinRead;
use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
use std::{
    fs,
    io::{Cursor, Read, Seek, SeekFrom},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

use bkhdd::{HDILayout, BLOCK_SIZE};

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
        .get_matches();

    info!("Starting");

    let image_name = matches.value_of("IMAGE_NAME").unwrap();

    let mut f = fs::File::open(image_name)?;
    f.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; BLOCK_SIZE as usize];
    f.read_exact(&mut buf)?;
    println!("len: {}, {:x?}", buf.len(), buf);
    let cs = buf[..511].iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
    println!("cs = {:x} cs_neg = {:x}", cs, -(cs as i8) as u8);
    // buf.iter_mut().for_each(|b| *b = !*b);
    // println!("{:x?}", buf);
    let mut c = Cursor::new(&buf);
    let ss = HDILayout::read(&mut c)?;
    let mut c = Cursor::new(&buf);

    dbg!(ss);

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
