use std::{env, error::Error, fs, path::PathBuf};

use pce::emulator::Emulator;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let mut rom_path: Option<PathBuf> = None;
    let mut load_backup: Option<PathBuf> = None;
    let mut save_backup: Option<PathBuf> = None;
    let mut load_bram: Option<PathBuf> = None;
    let mut save_bram: Option<PathBuf> = None;
    let mut frame_limit: Option<usize> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--load-backup" => {
                if let Some(path) = args.next() {
                    load_backup = Some(PathBuf::from(path));
                } else {
                    eprintln!("--load-backup requires a file path");
                    return Ok(());
                }
            }
            "--save-backup" => {
                if let Some(path) = args.next() {
                    save_backup = Some(PathBuf::from(path));
                } else {
                    eprintln!("--save-backup requires a file path");
                    return Ok(());
                }
            }
            "--load-bram" => {
                if let Some(path) = args.next() {
                    load_bram = Some(PathBuf::from(path));
                } else {
                    eprintln!("--load-bram requires a file path");
                    return Ok(());
                }
            }
            "--save-bram" => {
                if let Some(path) = args.next() {
                    save_bram = Some(PathBuf::from(path));
                } else {
                    eprintln!("--save-bram requires a file path");
                    return Ok(());
                }
            }
            "--frame-limit" => {
                if let Some(value) = args.next() {
                    match value.parse::<usize>() {
                        Ok(limit) => frame_limit = Some(limit),
                        Err(_) => {
                            eprintln!("invalid --frame-limit value: {value}");
                            return Ok(());
                        }
                    }
                } else {
                    eprintln!("--frame-limit requires a value");
                    return Ok(());
                }
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            _ if rom_path.is_none() => rom_path = Some(PathBuf::from(arg)),
            other => {
                eprintln!("Unknown argument: {other}");
                print_usage();
                return Ok(());
            }
        }
    }

    let rom_path = match rom_path {
        Some(path) => path,
        None => {
            print_usage();
            return Ok(());
        }
    };

    let rom = fs::read(&rom_path)?;

    let mut emulator = Emulator::new();
    let is_pce = rom_path
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);
    let default_backup = if is_pce {
        Some(rom_path.with_extension("sav"))
    } else {
        None
    };
    let default_bram = if is_pce {
        Some(rom_path.with_extension("brm"))
    } else {
        None
    };

    if is_pce {
        emulator.load_hucard(&rom)?;
        let backup_to_load = load_backup.or_else(|| {
            default_backup
                .as_ref()
                .filter(|path| path.exists())
                .cloned()
        });
        if let Some(load_path) = backup_to_load {
            match fs::read(&load_path) {
                Ok(bytes) => {
                    if let Err(err) = emulator.load_backup_ram(&bytes) {
                        eprintln!(
                            "warning: failed to load backup RAM from {}: {err}",
                            load_path.display()
                        );
                    }
                }
                Err(err) => eprintln!(
                    "warning: could not read backup RAM file {}: {err}",
                    load_path.display()
                ),
            }
        }
        let bram_to_load =
            load_bram.or_else(|| default_bram.as_ref().filter(|path| path.exists()).cloned());
        if let Some(load_path) = bram_to_load {
            match fs::read(&load_path) {
                Ok(bytes) => {
                    if let Err(err) = emulator.load_bram(&bytes) {
                        eprintln!(
                            "warning: failed to load BRAM from {}: {err}",
                            load_path.display()
                        );
                    }
                }
                Err(err) => eprintln!(
                    "warning: could not read BRAM file {}: {err}",
                    load_path.display()
                ),
            }
        }
    } else {
        emulator.load_program(0xC000, &rom);
    }
    emulator.reset();

    if let Some(limit) = frame_limit {
        const MAX_FRAME_BUDGET: u64 = 50_000_000;
        let mut frames = 0usize;
        let mut remaining = MAX_FRAME_BUDGET;
        while frames < limit && remaining > 0 {
            let cycles = emulator.tick() as u64;
            remaining = remaining.saturating_sub(cycles.max(1));
            if emulator.take_frame().is_some() {
                frames += 1;
            }
        }
        if frames < limit {
            eprintln!(
                "warning: collected {frames} / {limit} frames before exhausting cycle budget ({MAX_FRAME_BUDGET})."
            );
        } else {
            println!("collected {limit} frame(s) within budget");
        }
    } else {
        emulator.run_until_halt(Some(50_000));
    }

    println!(
        "Finished after {} cycles. A={:#04X} X={:#04X} Y={:#04X} PC={:#06X}",
        emulator.cycles(),
        emulator.cpu.a,
        emulator.cpu.x,
        emulator.cpu.y,
        emulator.cpu.pc,
    );

    if let Some(snapshot) = emulator.save_backup_ram() {
        let save_path = save_backup.or_else(|| default_backup.clone());
        if let Some(path) = save_path {
            if let Err(err) = fs::write(&path, snapshot) {
                eprintln!(
                    "warning: failed to write backup RAM to {}: {err}",
                    path.display()
                );
            }
        }
    } else if save_backup.is_some() {
        eprintln!("warning: no backup RAM present for this program; nothing saved");
    }

    if is_pce {
        let snapshot = emulator.save_bram();
        let save_path = save_bram.or_else(|| default_bram.clone());
        if let Some(path) = save_path {
            if let Err(err) = fs::write(&path, snapshot) {
                eprintln!("warning: failed to write BRAM to {}: {err}", path.display());
            }
        }
    }

    Ok(())
}

fn print_usage() {
    eprintln!(
        "Usage: pce <program.[bin|pce]> [--load-backup <file>] [--save-backup <file>] [--load-bram <file>] [--save-bram <file>]"
    );
    eprintln!("  .bin  : loads a raw HuC6280 program at $C000");
    eprintln!("  .pce  : loads a HuCard image and maps initial banks");
    eprintln!("Options:");
    eprintln!("  --load-backup <file>  Load HuCard backup RAM from file before reset");
    eprintln!("  --save-backup <file>  Save HuCard backup RAM to file after run");
    eprintln!("  --load-bram <file>    Load Ten no Koe 2 BRAM (2KB) from file before reset");
    eprintln!("  --save-bram <file>    Save Ten no Koe 2 BRAM (2KB) to file after run");
    eprintln!("  --frame-limit <n>     Run until N frames are produced (or budget exhausted)");
    eprintln!("  --help                Show this message");
    eprintln!();
    eprintln!(
        "When running a .pce HuCard, backup RAM automatically loads/saves from the \
         ROM path with .sav (cart RAM) and .brm (Ten no Koe 2 BRAM) extensions unless overridden."
    );
}
