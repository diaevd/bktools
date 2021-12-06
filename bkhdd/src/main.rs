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
    io::BinInvertedReader, AHDDLayout, HDILayout, AHDD_CYL_B, AHDD_LOGD_B, AHDD_PT_SEC, BLOCK_SIZE,
};

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
    let hdi = HDILayout::read(&mut f)?;
    dbg!(&hdi);
    let model = &hdi.model_name;
    // model.iter().swap_pairs();

    let mut rr = BinInvertedReader::new(&mut f);
    rr.seek(SeekFrom::Start(
        ((AHDD_PT_SEC + 1) * BLOCK_SIZE + AHDD_LOGD_B) as u64,
    ))?;

    let ap_h = AHDDLayout::read(&mut rr)?;
    dbg!(&ap_h);

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
