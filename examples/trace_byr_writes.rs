use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Enable VDC write logging
    emu.bus.vdc_enable_write_log(100);

    let mut frames = 0;
    let mut prev_byr = 0xFFFFu16;
    let mut total_ticks = 0u64;
    while frames < 10 {
        emu.tick();
        total_ticks += 1;

        let byr = emu.bus.vdc_register(0x08).unwrap_or(0);
        if byr != prev_byr {
            println!(
                "tick {}: BYR changed {} -> {} (R08=0x{:04X}) PC=???",
                total_ticks, prev_byr, byr, byr
            );
            prev_byr = byr;
        }

        if let Some(_) = emu.take_frame() {
            frames += 1;
            println!(
                "--- Frame {} completed (tick {}) BYR={} ---",
                frames, total_ticks, byr
            );
        }
    }

    // Check what value is being written to R08 (BYR)
    // Also check if there's a per-line BYR latch issue
    println!("\nFinal register values:");
    println!(
        "  R07 (BXR) = 0x{:04X}",
        emu.bus.vdc_register(0x07).unwrap_or(0)
    );
    println!(
        "  R08 (BYR) = 0x{:04X}",
        emu.bus.vdc_register(0x08).unwrap_or(0)
    );

    Ok(())
}
