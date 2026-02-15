use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Enable VRAM write logging to capture BAT writes
    // BAT for 64x64 map uses words 0-4095 (0x000-0xFFF)
    emu.bus.vdc_enable_write_log(50000);

    let mut frames = 0;
    while frames < 5 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
        }
    }

    let log = emu.bus.vdc_take_write_log();

    // Find writes of tile 0x3201 (entry for tile 201 palette 3)
    println!(
        "Searching for writes of 0x3201 (tile 201p3) in {} log entries:",
        log.len()
    );
    for &(addr, val) in &log {
        if val == 0x3201 {
            println!("  VRAM[{:#06X}] = {:#06X}", addr, val);
        }
    }

    // Also check what was written at flat address 0x205 and page address 0x105
    println!("\nWrites to VRAM[0x0105]:");
    for &(addr, val) in &log {
        if addr == 0x0105 {
            println!("  {:#06X}", val);
        }
    }
    println!("Writes to VRAM[0x0205]:");
    for &(addr, val) in &log {
        if addr == 0x0205 {
            println!("  {:#06X}", val);
        }
    }

    // Show first 20 writes to BAT area (addr < 0x1000)
    println!("\nFirst 20 BAT area writes (addr < 0x1000):");
    let mut count = 0;
    for &(addr, val) in &log {
        if addr < 0x1000 {
            println!("  VRAM[{:#06X}] = {:#06X}", addr, val);
            count += 1;
            if count >= 20 {
                break;
            }
        }
    }

    Ok(())
}
