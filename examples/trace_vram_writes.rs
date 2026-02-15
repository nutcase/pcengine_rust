use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Check VRAM writes to font tile area (tiles 0x130-0x160, VRAM 0x1300-0x1600)
    // Also check total VRAM write count
    let vram_writes_before = emu.bus.vdc_vram_data_high_writes();

    // Run 10 frames
    let mut frames = 0;
    while frames < 10 {
        emu.tick();
        if emu.take_frame().is_some() {
            frames += 1;
        }
    }

    let vram_writes_after_10 = emu.bus.vdc_vram_data_high_writes();
    println!(
        "VRAM high-byte writes in first 10 frames: {}",
        vram_writes_after_10 - vram_writes_before
    );

    // Check font tile area
    println!("\nFont tile area (0x1300-0x15FF) after 10 frames:");
    let mut nonzero_words = 0;
    for addr in 0x1300..0x1600u16 {
        let w = emu.bus.vdc_vram_word(addr);
        if w != 0 {
            nonzero_words += 1;
        }
    }
    println!("  Non-zero words: {} / {}", nonzero_words, 0x300);

    // Check a specific tile pattern - tile 0x148 'H'
    println!("\nTile 0x148 pattern after 10 frames:");
    let base = 0x148 * 16;
    for row in 0..8 {
        let w0 = emu.bus.vdc_vram_word((base + row) as u16);
        let w1 = emu.bus.vdc_vram_word((base + row + 8) as u16);
        let p0 = (w0 & 0xFF) as u8;
        let p1 = ((w0 >> 8) & 0xFF) as u8;
        let p2 = (w1 & 0xFF) as u8;
        let p3 = ((w1 >> 8) & 0xFF) as u8;
        print!("  ");
        for bit in (0..8).rev() {
            let px = ((p0 >> bit) & 1)
                | (((p1 >> bit) & 1) << 1)
                | (((p2 >> bit) & 1) << 2)
                | (((p3 >> bit) & 1) << 3);
            if px == 0 {
                print!(".");
            } else {
                print!("{:X}", px);
            }
        }
        println!("  ({:04X} {:04X})", w0, w1);
    }

    // Now run to 300 frames and check again
    while frames < 300 {
        emu.tick();
        if emu.take_frame().is_some() {
            frames += 1;
        }
    }

    let vram_writes_after_300 = emu.bus.vdc_vram_data_high_writes();
    println!(
        "\nTotal VRAM high-byte writes after 300 frames: {}",
        vram_writes_after_300
    );

    println!("\nTile 0x148 pattern after 300 frames:");
    let base = 0x148 * 16;
    for row in 0..8 {
        let w0 = emu.bus.vdc_vram_word((base + row) as u16);
        let w1 = emu.bus.vdc_vram_word((base + row + 8) as u16);
        let p0 = (w0 & 0xFF) as u8;
        let p1 = ((w0 >> 8) & 0xFF) as u8;
        let p2 = (w1 & 0xFF) as u8;
        let p3 = ((w1 >> 8) & 0xFF) as u8;
        print!("  ");
        for bit in (0..8).rev() {
            let px = ((p0 >> bit) & 1)
                | (((p1 >> bit) & 1) << 1)
                | (((p2 >> bit) & 1) << 2)
                | (((p3 >> bit) & 1) << 3);
            if px == 0 {
                print!(".");
            } else {
                print!("{:X}", px);
            }
        }
        println!("  ({:04X} {:04X})", w0, w1);
    }

    // Check total VRAM DMA operations
    println!("\nVRAM DMA count: {}", emu.bus.vdc_vram_dma_count());
    println!(
        "Last VRAM DMA source: {:04X}",
        emu.bus.vdc_vram_last_source()
    );
    println!(
        "Last VRAM DMA dest: {:04X}",
        emu.bus.vdc_vram_last_destination()
    );
    println!(
        "Last VRAM DMA length: {:04X}",
        emu.bus.vdc_vram_last_length()
    );

    // Check CRAM DMA
    println!("CRAM DMA count: {}", emu.bus.cram_dma_count());
    println!("Last CRAM source: {:04X}", emu.bus.vdc_cram_last_source());
    println!("Last CRAM length: {:04X}", emu.bus.vdc_cram_last_length());

    // Check how many VRAM writes went to the font area
    // We can check by looking at register write counts for R02 (VRAM write address low)
    println!("\nVDC register write counts:");
    let counts = emu.bus.vdc_register_write_counts();
    for (i, &count) in counts.iter().enumerate() {
        if count > 0 {
            println!("  R{:02X}: {} writes", i, count);
        }
    }

    // Check VRAM data write stats
    println!(
        "\nVRAM data low writes: {}",
        emu.bus.vdc_vram_data_low_writes()
    );
    println!(
        "VRAM data high writes: {}",
        emu.bus.vdc_vram_data_high_writes()
    );
    println!(
        "VRAM data high-without-low: {}",
        emu.bus.vdc_vram_data_high_without_low()
    );

    Ok(())
}
