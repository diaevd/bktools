use clap::{crate_authors, crate_name, crate_version, App, AppSettings, Arg};
use color_eyre::eyre::Result;
// use tracing::info;
use tracing_subscriber::EnvFilter;

use bkhdd::HDI;

fn main() -> Result<()> {
    setup_logging()?;

    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::DeriveDisplayOrder)
        .subcommand(
            App::new("info")
                .alias("show")
                .about("Disk image information")
                .arg(
                    Arg::new("IMAGE_NAME")
                        .required(true)
                        .help("Disk image file path"),
                ),
        )
        .subcommand(
            App::new("list").alias("ls").about("Partitions list").arg(
                Arg::new("IMAGE_NAME")
                    .required(true)
                    .help("Disk image file path"),
            ),
        )
        .get_matches();
    // dbg!(&matches);

    let cmd = matches.subcommand_name().unwrap();
    let image_name = matches
        .subcommand_matches(cmd)
        .unwrap()
        .value_of("IMAGE_NAME")
        .unwrap();

    // dbg!(&cmd, &image_name);

    let mut hdi = HDI::new(image_name);
    hdi.try_open()?;

    match cmd {
        "info" => {
            if hdi.is_hdi {
                println!("HDI Info:");
                let info = hdi.info();
                println!(
                    "\tC/H/S: {}/{}/{} Version: {}",
                    info.cylinders, info.heads, info.sectors, info.fw_version
                );
                println!(
                    "\tName: \"{}\" Serial: \"{}\"",
                    info.model_name, info.serial_number
                );
            }
            print!("Controller: ");
            if hdi.is_ahdd {
                println!("AltPro. Info:");
            }
            let parts = hdi.partitions();
            dbg!(parts);
        }
        _ => unreachable!(),
    }

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
