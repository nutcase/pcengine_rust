use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let mut frames = 0;
    while frames < 150 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
        }
    }

    println!("VDC register write counts after 150 frames:");
    let counts = emu.bus.vdc_register_write_counts();
    let reg_names = [
        "R00 MAWR", "R01 MARR", "R02 VWR", "R03 ???", "R04 ???", "R05 CR", "R06 RCR", "R07 BXR",
        "R08 BYR", "R09 MWR", "R0A HSR", "R0B HDR", "R0C VPR", "R0D VDW", "R0E VCR", "R0F DCR",
        "R10 SOUR", "R11 DESR", "R12 LENR", "R13 SATB",
    ];
    for (i, &count) in counts.iter().enumerate() {
        if count > 0 {
            let name = reg_names.get(i).unwrap_or(&"???");
            println!("  {}: {} writes", name, count);
        }
    }

    // Check MWR history: register 0x09
    let mwr_writes = counts[0x09];
    println!("\nMWR (R09) was written {} times", mwr_writes);
    println!(
        "Current MWR = {:#06X}",
        emu.bus.vdc_register(0x09).unwrap_or(0)
    );

    // Check what the game ACTUALLY writes for MAWR when populating title tiles
    // The title tile 0x3201 was written to VRAM[0x0105]
    // That means MAWR was set to 0x0105 at some point
    // MAWR is register 0x00
    println!("\nMAWR was written {} times", counts[0x00]);

    // Let's trace MAWR values in a specific range
    // Enable MAWR logging for the BAT range
    emu.bus.vdc_enable_mawr_log(0x0100, 0x0110);
    // Run a few more frames to capture title writes
    let mut more_frames = 0;
    while more_frames < 10 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            more_frames += 1;
        }
    }
    let mawr_log = emu.bus.vdc_take_mawr_log();
    println!("\nMAWR values set in range 0x100-0x10F during frames 151-160:");
    for &addr in &mawr_log {
        println!("  MAWR = {:#06X}", addr);
    }

    // Also check VRAM writes in that range
    emu.bus.vdc_enable_write_log(50000);
    more_frames = 0;
    while more_frames < 10 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            more_frames += 1;
        }
    }
    let write_log = emu.bus.vdc_take_write_log();
    println!("\nVRAM writes to BAT area (0x100-0x110) during frames 161-170:");
    for &(addr, val) in &write_log {
        if addr >= 0x0100 && addr < 0x0110 {
            let tile = val & 0x7FF;
            let pal = (val >> 12) & 0xF;
            println!(
                "  VRAM[{:#06X}] = {:#06X} (tile={:#05X} pal={})",
                addr, val, tile, pal
            );
        }
    }

    Ok(())
}
