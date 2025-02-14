use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

use clap::{Args, CommandFactory, Parser, Subcommand};
use espflash::{
    cli::{self, config::Config, monitor::monitor, *},
    error::Error,
    flasher::parse_partition_table,
    logging::initialize_logger,
    targets::{Chip, XtalFrequency},
    update::check_for_update,
};
use log::{debug, info, LevelFilter};
use miette::{IntoDiagnostic, Result, WrapErr};

#[derive(Debug, Parser)]
#[command(about, max_term_width = 100, propagate_version = true, version)]
pub struct Cli {
    #[command(subcommand)]
    subcommand: Commands,

    /// Do not check for updates
    #[clap(short = 'S', long, global = true, action)]
    skip_update_check: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print information about a connected target device
    ///
    /// Automatically detects and prints the chip type, crystal frequency, flash
    /// size, chip features, and MAC address of a connected target device.
    BoardInfo(ConnectArgs),
    /// Generate completions for the given shell
    ///
    /// The completions are printed to stdout, and can be redirected as needed.
    /// The directory in which completion scripts are stored differs
    /// depending on which shell is being used; consult your shell's
    /// documentation to determine the appropriate path.
    Completions(CompletionsArgs),
    /// Erase Flash entirely
    EraseFlash(EraseFlashArgs),
    /// Erase specified partitions
    EraseParts(ErasePartsArgs),
    /// Erase specified region
    EraseRegion(EraseRegionArgs),
    /// Flash an application in ELF format to a connected target device
    ///
    /// Given a path to an ELF file, first convert it into the appropriate
    /// binary application image format as required by the ESP32 devices. Once
    /// we have a valid application image, we can write the bootloader,
    /// partition table, and application image to the connected target device.
    ///
    /// Please refer to the ESP-IDF documentation for more information on the
    /// binary image format:
    ///
    /// https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/system/app_image_format.html
    Flash(FlashArgs),
    /// Hold the target device in reset
    HoldInReset(ConnectArgs),
    /// Open the serial monitor without flashing the connected target device
    Monitor(MonitorArgs),
    /// Convert partition tables between CSV and binary format
    ///
    /// Uses the ESP-IDF format for partition tables; please refer to the
    /// ESP-IDF documentation for more information on this format:
    ///
    /// https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/partition-tables.html
    ///
    /// Allows for conversion between formats via the '--to-csv' and
    /// '--to-binary' options, plus the ability to print a partition table
    /// in tabular format.
    PartitionTable(PartitionTableArgs),
    /// Read SPI flash content
    ReadFlash(ReadFlashArgs),
    /// Reset the target device
    Reset(ConnectArgs),
    /// Generate a binary application image and save it to a local disk
    ///
    /// If the '--merge' option is used, then the bootloader, partition table,
    /// and all application segments will be merged into a single binary file.
    /// Otherwise, each segment will be saved as individual binaries, prefixed
    /// with their intended addresses in flash.
    SaveImage(SaveImageArgs),
    /// Write a binary file to a specific address in a target device's flash
    WriteBin(WriteBinArgs),
    /// Calculate the MD5 checksum of the given region
    ChecksumMd5(ChecksumMd5Args),
}

/// Erase named partitions based on provided partition table
#[derive(Debug, Args)]
#[non_exhaustive]
pub struct ErasePartsArgs {
    /// Connection configuration
    #[clap(flatten)]
    pub connect_args: ConnectArgs,
    /// Labels of the partitions to be erased
    #[arg(value_name = "LABELS", value_delimiter = ',')]
    pub erase_parts: Vec<String>,
    /// Input partition table
    #[arg(long, value_name = "FILE")]
    pub partition_table: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[non_exhaustive]
struct FlashArgs {
    /// Connection configuration
    #[clap(flatten)]
    connect_args: ConnectArgs,
    /// Flashing configuration
    #[clap(flatten)]
    pub flash_config_args: FlashConfigArgs,
    /// Flashing arguments
    #[clap(flatten)]
    flash_args: cli::FlashArgs,
    /// ELF image to flash
    image: PathBuf,
}

#[derive(Debug, Args)]
#[non_exhaustive]
struct SaveImageArgs {
    /// ELF image
    image: PathBuf,
    /// Flashing configuration
    #[clap(flatten)]
    pub flash_config_args: FlashConfigArgs,
    /// Sage image arguments
    #[clap(flatten)]
    save_image_args: cli::SaveImageArgs,
}

/// Writes a binary file to a specific address in the chip's flash
#[derive(Debug, Args)]
#[non_exhaustive]
struct WriteBinArgs {
    /// Address at which to write the binary file
    #[arg(value_parser = parse_u32)]
    pub address: u32,
    /// File containing the binary data to write
    pub file: String,
    /// Connection configuration
    #[clap(flatten)]
    connect_args: ConnectArgs,
}

fn main() -> Result<()> {
    miette::set_panic_hook();
    initialize_logger(LevelFilter::Info);

    // Attempt to parse any provided comand-line arguments, or print the help
    // message and terminate if the invocation is not correct.
    let cli = Cli::parse();
    let args = cli.subcommand;
    debug!("{:#?}, {:#?}", args, cli.skip_update_check);

    // Only check for updates once the command-line arguments have been processed,
    // to avoid printing any update notifications when the help message is
    // displayed.
    if !cli.skip_update_check {
        check_for_update(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    }

    // Load any user configuration, if present.
    let config = Config::load()?;

    // Execute the correct action based on the provided subcommand and its
    // associated arguments.
    match args {
        Commands::BoardInfo(args) => board_info(&args, &config),
        Commands::Completions(args) => completions(&args, &mut Cli::command(), "espflash"),
        Commands::EraseFlash(args) => erase_flash(args, &config),
        Commands::EraseParts(args) => erase_parts(args, &config),
        Commands::EraseRegion(args) => erase_region(args, &config),
        Commands::Flash(args) => flash(args, &config),
        Commands::HoldInReset(args) => hold_in_reset(args, &config),
        Commands::Monitor(args) => serial_monitor(args, &config),
        Commands::PartitionTable(args) => partition_table(args),
        Commands::ReadFlash(args) => read_flash(args, &config),
        Commands::Reset(args) => reset(args, &config),
        Commands::SaveImage(args) => save_image(args, &config),
        Commands::WriteBin(args) => write_bin(args, &config),
        Commands::ChecksumMd5(args) => checksum_md5(&args, &config),
    }
}

pub fn erase_parts(args: ErasePartsArgs, config: &Config) -> Result<()> {
    if args.connect_args.no_stub {
        return Err(Error::StubRequired.into());
    }

    let mut flasher = connect(&args.connect_args, config, false, false)?;
    let partition_table = match args.partition_table {
        Some(path) => Some(parse_partition_table(&path)?),
        None => None,
    };

    info!("Erasing the following partitions: {:?}", args.erase_parts);

    erase_partitions(&mut flasher, partition_table, Some(args.erase_parts), None)?;
    flasher
        .connection()
        .reset_after(!args.connect_args.no_stub)?;

    info!("Specified partitions successfully erased!");

    Ok(())
}

fn reset(args: ConnectArgs, config: &Config) -> Result<()> {
    let mut args = args.clone();
    args.no_stub = true;
    let mut flash = connect(&args, config, true, true)?;
    info!("Resetting target device");
    flash.connection().reset()?;

    Ok(())
}

fn hold_in_reset(args: ConnectArgs, config: &Config) -> Result<()> {
    connect(&args, config, true, true)?;
    info!("Holding target device in reset");

    Ok(())
}

fn flash(args: FlashArgs, config: &Config) -> Result<()> {
    let mut flasher = connect(
        &args.connect_args,
        config,
        args.flash_args.no_verify,
        args.flash_args.no_skip,
    )?;
    flasher.verify_minimum_revision(args.flash_args.image.min_chip_rev)?;

    // If the user has provided a flash size via a command-line argument, we'll
    // override the detected (or default) value with this.
    if let Some(flash_size) = args.flash_config_args.flash_size {
        flasher.set_flash_size(flash_size);
    } else if let Some(flash_size) = config.flash.size {
        flasher.set_flash_size(flash_size);
    }

    print_board_info(&mut flasher)?;

    let chip = flasher.chip();
    let target = chip.into_target();
    let target_xtal_freq = target.crystal_freq(flasher.connection())?;

    // Read the ELF data from the build path and load it to the target.
    let elf_data = fs::read(&args.image).into_diagnostic()?;

    if args.flash_args.ram {
        flasher.load_elf_to_ram(&elf_data, Some(&mut EspflashProgress::default()))?;
    } else {
        let flash_data = make_flash_data(
            args.flash_args.image,
            &args.flash_config_args,
            config,
            None,
            None,
        )?;

        if args.flash_args.erase_parts.is_some() || args.flash_args.erase_data_parts.is_some() {
            erase_partitions(
                &mut flasher,
                flash_data.partition_table.clone(),
                args.flash_args.erase_parts,
                args.flash_args.erase_data_parts,
            )?;
        }

        flash_elf_image(&mut flasher, &elf_data, flash_data, target_xtal_freq)?;
    }

    if args.flash_args.monitor {
        let pid = flasher.get_usb_pid()?;
        let mut monitor_args = args.flash_args.monitor_args;

        // The 26MHz ESP32-C2's need to be treated as a special case.
        if chip == Chip::Esp32c2
            && target_xtal_freq == XtalFrequency::_26Mhz
            && monitor_args.monitor_baud == 115_200
        {
            // 115_200 * 26 MHz / 40 MHz = 74_880
            monitor_args.monitor_baud = 74_880;
        }

        monitor_args.elf = Some(args.image);

        monitor(flasher.into_serial(), Some(&elf_data), pid, monitor_args)
    } else {
        Ok(())
    }
}

fn save_image(args: SaveImageArgs, config: &Config) -> Result<()> {
    let elf_data = fs::read(&args.image)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to open image {}", args.image.display()))?;

    // Since we have no `Flasher` instance and as such cannot print the board
    // information, we will print whatever information we _do_ have.
    println!("Chip type:         {}", args.save_image_args.chip);
    println!("Merge:             {}", args.save_image_args.merge);
    println!("Skip padding:      {}", args.save_image_args.skip_padding);

    let flash_data = make_flash_data(
        args.save_image_args.image,
        &args.flash_config_args,
        config,
        None,
        None,
    )?;

    let xtal_freq = args
        .save_image_args
        .xtal_freq
        .unwrap_or(XtalFrequency::default(args.save_image_args.chip));

    save_elf_as_image(
        &elf_data,
        args.save_image_args.chip,
        args.save_image_args.file,
        flash_data,
        args.save_image_args.merge,
        args.save_image_args.skip_padding,
        xtal_freq,
    )?;

    Ok(())
}

fn write_bin(args: WriteBinArgs, config: &Config) -> Result<()> {
    let mut flasher = connect(&args.connect_args, config, false, false)?;
    print_board_info(&mut flasher)?;

    let mut f = File::open(&args.file).into_diagnostic()?;
    let size = f.metadata().into_diagnostic()?.len();
    let mut buffer = Vec::with_capacity(size.try_into().into_diagnostic()?);
    f.read_to_end(&mut buffer).into_diagnostic()?;

    flasher.write_bin_to_flash(
        args.address,
        &buffer,
        Some(&mut EspflashProgress::default()),
    )?;

    Ok(())
}
