use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Enable VRAM write logging to capture all writes
    emu.bus.vdc_enable_write_log(200000);

    let mut frames = 0;
    let mut prev_mwr = emu.bus.vdc_register(0x09).unwrap_or(0xFFFF);
    println!("Initial MWR = {:#06X}", prev_mwr);

    while frames < 150 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
            let mwr = emu.bus.vdc_register(0x09).unwrap_or(0xFFFF);
            if mwr != prev_mwr {
                println!(
                    "Frame {}: MWR changed {:#06X} -> {:#06X}",
                    frames, prev_mwr, mwr
                );
                prev_mwr = mwr;
            }
            if frames <= 5 || frames % 50 == 0 {
                let cr = emu.bus.vdc_register(0x05).unwrap_or(0);
                let inc_bits = (cr >> 11) & 3;
                let inc_step = match inc_bits {
                    0 => 1,
                    1 => 32,
                    2 => 64,
                    _ => 128,
                };
                println!(
                    "Frame {}: MWR={:#06X} CR={:#06X} inc_step={}",
                    frames, mwr, cr, inc_step
                );
            }
        }
    }

    let log = emu.bus.vdc_take_write_log();
    println!("\nTotal VRAM writes logged: {}", log.len());

    // Check where tile 0x3201 appears in VRAM
    println!("\nSearching for writes of tile 0x3201:");
    for &(addr, val) in &log {
        if val == 0x3201 {
            println!("  VRAM[{:#06X}] = {:#06X}", addr, val);
        }
    }

    // Check both addresses
    let v105 = emu.bus.vdc_vram_word(0x0105);
    let v205 = emu.bus.vdc_vram_word(0x0205);
    println!("\nFinal VRAM state:");
    println!("  VRAM[0x0105] = {:#06X}  (32-stride row=8,col=5)", v105);
    println!("  VRAM[0x0205] = {:#06X}  (64-stride row=8,col=5)", v205);

    // Dump BAT row 8 with both strides to see which pattern makes sense
    println!("\nBAT row 8 with stride 32 (addr = 8*32+col = 0x100+col):");
    let mut line = String::new();
    for col in 0..32 {
        let addr = 8 * 32 + col;
        let val = emu.bus.vdc_vram_word(addr as u16);
        if col < 16 {
            line.push_str(&format!("{:04X} ", val));
        }
    }
    println!("  cols 0-15: {}", line);

    println!("\nBAT row 8 with stride 64 (addr = 8*64+col = 0x200+col):");
    let mut line = String::new();
    for col in 0..32 {
        let addr = 8 * 64 + col;
        let val = emu.bus.vdc_vram_word(addr as u16);
        if col < 16 {
            line.push_str(&format!("{:04X} ", val));
        }
    }
    println!("  cols 0-15: {}", line);

    Ok(())
}
