use std::{thread::sleep, time::Duration};

use anyhow::Result;

use clap::{Parser, Subcommand};
use hxdmp::hexdump;

use wchisp::{
    constants::SECTOR_SIZE,
    transport::{SerialTransport, Transport, UsbTransport},
    Baudrate, Flashing,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[clap(group(clap::ArgGroup::new("transport").args(&["usb", "serial"])))]
struct Cli {
    /// Turn debugging information on
    #[arg(long = "verbose", short = 'v')]
    debug: bool,

    /// Use the USB transport layer
    #[arg(long, short, default_value_t = true, default_value_if("serial", clap::builder::ArgPredicate::IsPresent, "false"), conflicts_with_all = ["serial", "port", "baudrate"])]
    usb: bool,

    /// Use the Serial transport layer
    #[arg(long, short, conflicts_with_all = ["usb", "device"])]
    serial: bool,

    /// Optional USB device index to operate on
    #[arg(long, short, value_name = "INDEX", default_value = None, requires = "usb")]
    device: Option<usize>,

    /// Select the serial port
    #[arg(long, short, requires = "serial")]
    port: Option<String>,

    /// Select the serial baudrate
    #[arg(long, short, ignore_case = true, value_enum, requires = "serial")]
    baudrate: Option<Baudrate>,

    /// Retry scan for certain seconds, helpful on slow USB devices
    #[arg(long, short, default_value = "0")]
    retry: u32,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Probe any connected devices
    Probe {},
    /// Get info about current connected chip
    Info {
        /// Chip name(prefix) check
        #[arg(long)]
        chip: Option<String>,
    },
    /// Reset the target connected
    Reset {},
    /// Erase code flash
    Erase {},
    /// Download to code flash and reset
    Flash {
        /// The path to the file to be downloaded to the code flash
        path: String,
        /// Do not erase the code flash before flashing
        #[clap(short = 'E', long)]
        no_erase: bool,
        /// Do not verify the code flash after flashing
        #[clap(short = 'V', long)]
        no_verify: bool,
        /// Do not reset the target after flashing
        #[clap(short = 'R', long)]
        no_reset: bool,
    },
    /// Verify code flash content
    Verify { path: String },
    /// EEPROM(data flash) operations
    Eeprom {
        #[command(subcommand)]
        command: Option<EepromCommands>,
    },
    /// Config CFG register
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,
    },
    /// kmbox device operations (custom ISP protocol)
    Kmbox {
        #[command(subcommand)]
        command: KmboxCommands,
    },
    /// Fuzz ISP commands to discover hidden functionality
    Fuzz {
        /// Starting command byte (default: 0x80)
        #[arg(long, default_value = "0x80")]
        start: String,
        /// Ending command byte (default: 0x8f)
        #[arg(long, default_value = "0x8f")]
        end: String,
        /// Show verbose output for each command tested
        #[arg(long, short)]
        verbose: bool,
        /// Include already-known kmbox commands (0x80-0x83)
        #[arg(long)]
        include_known: bool,
    },
}

#[derive(Subcommand)]
enum KmboxCommands {
    /// Flash firmware to kmbox device
    Flash {
        /// Path to firmware file (.bin)
        path: String,
        /// Skip verification step
        #[arg(long)]
        no_verify: bool,
    },
    /// Read firmware from kmbox device (if read command exists)
    Read {
        /// Output path for firmware dump
        #[arg(long, short)]
        output: String,
        /// Number of bytes to read
        #[arg(long, short, default_value = "118784")]
        size: u32,
    },
    /// Probe kmbox device
    Probe {},
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Dump config register info
    Info {},
    /// Reset config register to default
    Reset {},
    /// Enable SWD mode(simulation mode)
    EnableDebug {},
    /// Disable SWD mode(simulation mode)
    DisableDebug {},
    /// Set config register to new value
    Set {
        /// New value of the config register
        #[arg(value_name = "HEX")]
        value: String,
    },
    /// Unprotect code flash
    Unprotect {},
}

#[derive(Subcommand)]
enum EepromCommands {
    /// Dump EEPROM data
    Dump {
        /// The path of the file to be written to
        path: Option<String>,
    },
    /// Erase EEPROM data
    Erase {},
    /// Programming EEPROM data
    Write {
        /// The path to the file to be downloaded to the data flash
        path: String,
        /// Do not erase the data flash before programming
        #[clap(short = 'E', long)]
        no_erase: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.debug {
        let _ = simplelog::TermLogger::init(
            simplelog::LevelFilter::Debug,
            simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        );
    } else {
        let _ = simplelog::TermLogger::init(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        );
    }

    if cli.retry > 0 {
        if !cli.usb && !cli.serial {
            log::warn!("No transport method specified (--usb or --serial); skipping retry logic.");
        } else {
            log::info!("Retrying scan for {} seconds", cli.retry);
            let start_time = std::time::Instant::now();
            while start_time.elapsed().as_secs() < cli.retry as u64 {
                if cli.usb {
                    let ndevices = UsbTransport::scan_devices()?;
                    if ndevices > 0 {
                        break;
                    }
                } else if cli.serial {
                    let ports = SerialTransport::scan_ports()?;
                    if !ports.is_empty() {
                        break;
                    }
                }
                sleep(Duration::from_millis(100));
            }
        }
    }

    match &cli.command {
        None | Some(Commands::Probe {}) => {
            if cli.usb {
                let ndevices = UsbTransport::scan_devices()?;
                log::info!(
                    "Found {ndevices} USB device{}",
                    match ndevices {
                        1 => "",
                        _ => "s",
                    }
                );
                for i in 0..ndevices {
                    let mut trans = UsbTransport::open_nth(i)?;
                    let chip = Flashing::get_chip(&mut trans)?;
                    log::info!("\tDevice #{i}: {chip}");
                }
            }
            if cli.serial {
                let ports = SerialTransport::scan_ports()?;
                let port_len = ports.len();
                log::info!(
                    "Found {port_len} serial port{}:",
                    match port_len {
                        1 => "",
                        _ => "s",
                    }
                );
                for p in ports {
                    log::info!("\t{p}");
                }
            }

            log::info!("hint: use `wchisp info` to check chip info");
        }
        Some(Commands::Info { chip }) => {
            let mut flashing = get_flashing(&cli)?;

            if let Some(expected_chip_name) = chip {
                flashing.check_chip_name(&expected_chip_name)?;
            }
            flashing.dump_info()?;
        }
        Some(Commands::Reset {}) => {
            let mut flashing = get_flashing(&cli)?;

            let _ = flashing.reset();
        }
        Some(Commands::Erase {}) => {
            let mut flashing = get_flashing(&cli)?;

            let sectors = flashing.chip.flash_size / 1024;
            flashing.erase_code(sectors)?;
        }
        // WRITE_CONFIG => READ_CONFIG => ISP_KEY => ERASE => PROGRAM => VERIFY => RESET
        Some(Commands::Flash {
            path,
            no_erase,
            no_verify,
            no_reset,
        }) => {
            let mut flashing = get_flashing(&cli)?;

            flashing.dump_info()?;

            let mut binary = wchisp::format::read_firmware_from_file(path)?;
            extend_firmware_to_sector_boundary(&mut binary);
            log::info!("Firmware size: {}", binary.len());

            if *no_erase {
                log::warn!("Skipping erase");
            } else {
                log::info!("Erasing...");
                let sectors = binary.len() / SECTOR_SIZE + 1;
                flashing.erase_code(sectors as u32)?;

                sleep(Duration::from_secs(1));
                log::info!("Erase done");
            }

            log::info!("Writing to code flash...");
            flashing.flash(&binary)?;
            sleep(Duration::from_millis(500));

            if *no_verify {
                log::warn!("Skipping verify");
            } else {
                log::info!("Verifying...");
                flashing.verify(&binary)?;
                log::info!("Verify OK");
            }

            if *no_reset {
                log::warn!("Skipping reset");
            } else {
                log::info!("Now reset device and skip any communication errors");
                let _ = flashing.reset();
            }
        }
        Some(Commands::Verify { path }) => {
            let mut flashing = get_flashing(&cli)?;

            let mut binary = wchisp::format::read_firmware_from_file(path)?;
            extend_firmware_to_sector_boundary(&mut binary);
            log::info!("Firmware size: {}", binary.len());
            log::info!("Verifying...");
            flashing.verify(&binary)?;
            log::info!("Verify OK");
        }
        Some(Commands::Eeprom { command }) => {
            let mut flashing = get_flashing(&cli)?;

            match command {
                None | Some(EepromCommands::Dump { .. }) => {
                    flashing.reidentify()?;

                    log::info!("Reading EEPROM(Data Flash)...");

                    let eeprom = flashing.dump_eeprom()?;
                    log::info!("EEPROM data size: {}", eeprom.len());

                    if let Some(EepromCommands::Dump {
                        path: Some(path),
                    }) = command
                    {
                        std::fs::write(path, eeprom)?;
                        log::info!("EEPROM data saved to {}", path);
                    } else {
                        let mut buf = vec![];
                        hexdump(&eeprom, &mut buf)?;
                        println!("{}", String::from_utf8_lossy(&buf));
                    }
                }
                Some(EepromCommands::Erase {}) => {
                    flashing.reidentify()?;

                    log::info!("Erasing EEPROM(Data Flash)...");
                    flashing.erase_data()?;
                    log::info!("EEPROM erased");
                }
                Some(EepromCommands::Write { path, no_erase }) => {
                    flashing.reidentify()?;

                    if *no_erase {
                        log::warn!("Skipping erase");
                    } else {
                        log::info!("Erasing EEPROM(Data Flash)...");
                        flashing.erase_data()?;
                        log::info!("EEPROM erased");
                    }

                    let eeprom = std::fs::read(path)?;
                    log::info!("Read {} bytes from bin file", eeprom.len());
                    if eeprom.len() as u32 != flashing.chip.eeprom_size {
                        anyhow::bail!(
                            "EEPROM size mismatch: expected {}, got {}",
                            flashing.chip.eeprom_size,
                            eeprom.len()
                        );
                    }

                    log::info!("Writing EEPROM(Data Flash)...");
                    flashing.write_eeprom(&eeprom)?;
                    log::info!("EEPROM written");
                }
            }
        }
        Some(Commands::Config { command }) => {
            let mut flashing = get_flashing(&cli)?;

            match command {
                None | Some(ConfigCommands::Info {}) => {
                    flashing.dump_config()?;
                }
                Some(ConfigCommands::Reset {}) => {
                    flashing.reset_config()?;
                    log::info!(
                        "Config register restored to default value(non-protected, debug-enabled)"
                    );
                }
                Some(ConfigCommands::EnableDebug {}) => {
                    flashing.enable_debug()?;
                    log::info!("Debug mode enabled");
                }
                Some(ConfigCommands::DisableDebug {}) => {
                    flashing.disable_debug()?;
                    log::info!("Debug mode disabled");
                }
                Some(ConfigCommands::Set { value }) => {
                    // flashing.write_config(value)?;
                    log::info!("setting cfg value {}", value);
                    unimplemented!()
                }
                Some(ConfigCommands::Unprotect {}) => {
                    flashing.unprotect(true)?;
                }
            }
        }
        Some(Commands::Kmbox { command }) => {
            handle_kmbox_command(&cli, command)?;
        }
        Some(Commands::Fuzz {
            start,
            end,
            verbose,
            include_known,
        }) => {
            handle_fuzz_command(&cli, start, end, *verbose, *include_known)?;
        }
    }

    Ok(())
}

/// Handle kmbox-specific commands
fn handle_kmbox_command(cli: &Cli, command: &KmboxCommands) -> Result<()> {
    use wchisp::protocol::Command;

    match command {
        KmboxCommands::Probe {} => {
            log::info!("Probing kmbox device...");

            if cli.usb {
                let ndevices = UsbTransport::scan_devices()?;
                if ndevices == 0 {
                    log::error!("No USB devices found");
                    return Ok(());
                }

                let device_index = cli.device.unwrap_or(0);
                let mut trans = UsbTransport::open_nth(device_index)?;

                log::info!("Sending kmbox init command (0x81)...");
                let cmd = Command::kmbox_init();
                let response = trans.transfer(cmd)?;

                log::info!("Response: {:02x?}", response.payload());

                if response.payload().len() >= 2 && response.payload()[0] == 0x00 {
                    log::info!("✓ kmbox device responded successfully!");
                    log::info!("  Status: 0x{:02x} 0x{:02x}", response.payload()[0], response.payload()[1]);
                } else {
                    log::warn!("⚠ Unexpected response from device");
                }
            } else {
                log::error!("kmbox commands currently only support USB transport");
            }
        }
        KmboxCommands::Flash { path, no_verify: _ } => {
            log::info!("Flashing kmbox firmware from: {}", path);
            log::warn!("kmbox flash not yet implemented - use standard wchisp flash for now");
            // TODO: Implement kmbox flashing
            unimplemented!("kmbox flash command");
        }
        KmboxCommands::Read { output, size } => {
            log::info!("Attempting to read {} bytes from kmbox...", size);
            log::info!("Will save to: {}", output);

            if cli.usb {
                let device_index = cli.device.unwrap_or(0);
                let mut trans = UsbTransport::open_nth(device_index)?;

                log::info!("Testing potential read commands...");

                // Try potential read commands
                let read_commands = [0x85, 0x86, 0x87, 0x8A];

                for cmd_byte in read_commands {
                    log::info!("  Trying command 0x{:02x}...", cmd_byte);

                    // Try: cmd len addr_lo addr_hi
                    let test_data = vec![cmd_byte, 0x3C, 0x00, 0x00];
                    let cmd = Command::kmbox_raw(cmd_byte, test_data[1..].to_vec());

                    match trans.transfer(cmd) {
                        Ok(response) => {
                            log::info!("    Response ({}bytes): {:02x?}",
                                response.payload().len(),
                                &response.payload()[..std::cmp::min(16, response.payload().len())]);

                            if response.payload().len() > 2 {
                                log::info!("    ⭐ Got data! This might be a read command!");
                            }
                        }
                        Err(e) => {
                            log::warn!("    Error: {}", e);
                        }
                    }
                }
            } else {
                log::error!("kmbox commands currently only support USB transport");
            }
        }
    }

    Ok(())
}

/// Handle ISP command fuzzing
fn handle_fuzz_command(
    cli: &Cli,
    start: &str,
    end: &str,
    verbose: bool,
    include_known: bool,
) -> Result<()> {
    use wchisp::protocol::Command;

    // Parse hex strings
    let start_byte = if start.starts_with("0x") || start.starts_with("0X") {
        u8::from_str_radix(&start[2..], 16)?
    } else {
        start.parse::<u8>()?
    };

    let end_byte = if end.starts_with("0x") || end.starts_with("0X") {
        u8::from_str_radix(&end[2..], 16)?
    } else {
        end.parse::<u8>()?
    };

    log::info!("Fuzzing ISP commands from 0x{:02x} to 0x{:02x}", start_byte, end_byte);
    log::warn!("⚠ This will send many commands to the device. Make sure you understand the risks!");
    if !include_known {
        log::info!("Skipping known kmbox session commands 0x80-0x83; pass --include-known to test them");
    }

    if !cli.usb {
        log::error!("Fuzzing currently only supports USB transport");
        return Ok(());
    }

    let device_index = cli.device.unwrap_or(0);
    let mut trans = UsbTransport::open_nth(device_index)?;

    log::info!("Connected to device");
    log::info!("Starting fuzzing...\n");

    let mut successful_commands = Vec::new();

    for cmd_byte in start_byte..=end_byte {
        if !include_known && known_kmbox_command_name(cmd_byte).is_some() {
            if verbose {
                log::info!(
                    "[0x{:02x}] Skipping known kmbox command ({})",
                    cmd_byte,
                    known_kmbox_command_name(cmd_byte).unwrap()
                );
            }
            continue;
        }

        // Test formats:
        // Format 1: cmd 02 00 00 (like kmbox init/end)
        // Format 2: cmd 3C 00 00 (like kmbox write/verify)

        let test_formats = [
            vec![0x02, 0x00, 0x00],  // Short command
            vec![0x3C, 0x00, 0x00],  // Medium command with address
        ];

        for (fmt_idx, test_data) in test_formats.iter().enumerate() {
            if verbose {
                log::info!("[0x{:02x}] Format {}: Testing...", cmd_byte, fmt_idx + 1);
            }

            let cmd = Command::kmbox_raw(cmd_byte, test_data.clone());

            match trans.transfer(cmd) {
                Ok(response) => {
                    let resp_len = response.payload().len();

                    // Format response bytes for display
                    let status = if resp_len >= 2 {
                        format!("{:02x} {:02x}", response.payload()[0], response.payload()[1])
                    } else if resp_len >= 1 {
                        format!("{:02x}", response.payload()[0])
                    } else {
                        "empty".to_string()
                    };

                    // Check if response is different from standard "00 00"
                    let is_interesting = resp_len != 2
                        || response.payload()[0] != 0x00
                        || response.payload()[1] != 0x00;

                    // Always print in verbose mode, or if interesting
                    if verbose {
                        // Verbose: show all responses with full hex dump
                        let hex_preview = if resp_len > 2 {
                            format!(" [{}]",
                                response.payload()[..resp_len.min(16)]
                                    .iter()
                                    .map(|b| format!("{:02x}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        } else {
                            String::new()
                        };

                        log::info!("  [0x{:02x}] fmt{} → {} bytes: {}{}{}",
                            cmd_byte,
                            fmt_idx + 1,
                            resp_len,
                            status,
                            hex_preview,
                            if is_interesting { " ⭐" } else { "" }
                        );
                    } else if is_interesting {
                        // Non-verbose: only show interesting responses
                        log::info!("  [0x{:02x}] fmt{} → {} bytes: {} ⭐ INTERESTING!",
                            cmd_byte,
                            fmt_idx + 1,
                            resp_len,
                            status
                        );
                    }

                    if is_interesting {
                        successful_commands.push((cmd_byte, fmt_idx + 1, resp_len));
                    }
                }
                Err(e) => {
                    if verbose {
                        log::debug!("  [0x{:02x}] fmt{} → Error: {}", cmd_byte, fmt_idx + 1, e);
                    }
                }
            }

            // Small delay to avoid overwhelming the device
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    log::info!("\n{}", "=".repeat(80));
    log::info!("Fuzzing complete!");
    log::info!("{}", "=".repeat(80));

    if successful_commands.is_empty() {
        log::info!("No interesting commands found (all returned standard 00 00 or failed)");
    } else {
        log::info!("Found {} potentially interesting command(s):", successful_commands.len());
        for (cmd, fmt, resp_len) in successful_commands {
            log::info!("  0x{:02x} (format {}) - {} byte response", cmd, fmt, resp_len);
        }
        log::info!("\nRe-run with --verbose to see full details");
    }

    Ok(())
}

fn known_kmbox_command_name(cmd: u8) -> Option<&'static str> {
    match cmd {
        0x80 => Some("write"),
        0x81 => Some("init"),
        0x82 => Some("verify"),
        0x83 => Some("end"),
        _ => None,
    }
}

fn extend_firmware_to_sector_boundary(buf: &mut Vec<u8>) {
    if buf.len() % 1024 != 0 {
        let remain = 1024 - (buf.len() % 1024);
        buf.extend_from_slice(&vec![0; remain]);
    }
}

fn get_flashing(cli: &Cli) -> Result<Flashing<'_>> {
    if cli.usb {
        Flashing::new_from_usb(cli.device)
    } else if cli.serial {
        Flashing::new_from_serial(cli.port.as_deref(), cli.baudrate)
    } else {
        unreachable!("No transport specified");
    }
}
