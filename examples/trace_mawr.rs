use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Enable VRAM write logging from the very start
    emu.bus.vdc_enable_write_log(200000);

    let mut frames = 0;
    while frames < 150 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
        }
    }

    let log = emu.bus.vdc_take_write_log();
    println!("Total VRAM writes: {}", log.len());

    // Find all writes where the value looks like a BAT entry with palette 3
    println!("\nWrites of BAT entries with palette 3 (0x3xxx) to BAT area:");
    let mut title_writes: Vec<(u16, u16)> = Vec::new();
    for &(addr, val) in &log {
        if (val >> 12) == 3 && addr < 0x1000 {
            title_writes.push((addr, val));
        }
    }

    for (i, &(addr, val)) in title_writes.iter().take(30).enumerate() {
        let tile = val & 0x7FF;
        let row_s32 = addr / 32;
        let col_s32 = addr % 32;
        let row_s64 = addr / 64;
        let col_s64 = addr % 64;
        println!(
            "  [{}] VRAM[{:#06X}] = {:#06X} (tile={:#05X}) stride32=({},{}) stride64=({},{})",
            i, addr, val, tile, row_s32, col_s32, row_s64, col_s64
        );
    }

    // Show what rows (stride 32 vs 64) have title tiles
    println!("\nTitle tile rows (stride 32):");
    let mut rows32: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for &(addr, _) in &title_writes {
        *rows32.entry(addr / 32).or_insert(0) += 1;
    }
    for (row, count) in &rows32 {
        println!("  row {}: {} tiles", row, count);
    }

    println!("\nTitle tile rows (stride 64):");
    let mut rows64: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for &(addr, _) in &title_writes {
        *rows64.entry(addr / 64).or_insert(0) += 1;
    }
    for (row, count) in &rows64 {
        println!("  row {}: {} tiles", row, count);
    }

    // Find the first write to BAT area (addr < 0x1000)
    println!("\nFirst 10 writes to BAT area (addr < 0x1000):");
    let mut bat_count = 0;
    for (i, &(addr, val)) in log.iter().enumerate() {
        if addr < 0x1000 {
            println!("  [{}] VRAM[{:#06X}] = {:#06X}", i, addr, val);
            bat_count += 1;
            if bat_count >= 10 {
                break;
            }
        }
    }

    // Find when 0x0200 first appears as a write value
    println!("\nFirst write of value 0x0200:");
    for (i, &(addr, val)) in log.iter().enumerate() {
        if val == 0x0200 {
            println!("  [{}] VRAM[{:#06X}] = {:#06X}", i, addr, val);
            break;
        }
    }

    // Count writes to BAT area vs character area
    let bat_writes = log.iter().filter(|&&(a, _)| a < 0x1000).count();
    let char_writes = log
        .iter()
        .filter(|&&(a, _)| a >= 0x1000 && a < 0x7000)
        .count();
    let sat_writes = log
        .iter()
        .filter(|&&(a, _)| a >= 0x1000 && a < 0x1100)
        .count();
    println!("\nWrite distribution:");
    println!("  BAT area (0x000-0xFFF): {} writes", bat_writes);
    println!("  Character area (0x1000-0x6FFF): {} writes", char_writes);
    println!("  SAT area (0x1000-0x10FF): {} writes", sat_writes);

    // Show write pattern around address 0x000-0x040
    println!("\nAll writes to 0x000-0x060:");
    for (i, &(addr, val)) in log.iter().enumerate() {
        if addr < 0x060 {
            println!("  [{}] VRAM[{:#06X}] = {:#06X}", i, addr, val);
        }
    }

    Ok(())
}
