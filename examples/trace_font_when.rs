use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let check_tiles: &[(char, u16)] = &[('0', 0x130), ('H', 0x148), ('I', 0x149), ('U', 0x155)];

    let mut frames = 0;
    let checkpoints = [1, 2, 3, 5, 10, 20, 50, 100, 150, 200, 250, 300];
    let mut cp_idx = 0;

    while cp_idx < checkpoints.len() {
        emu.tick();
        if emu.take_frame().is_some() {
            frames += 1;
            if frames == checkpoints[cp_idx] {
                println!("=== Frame {} ===", frames);
                for &(ch, tid) in check_tiles {
                    let base = tid as usize * 16;
                    let mut all_zero = true;
                    for w in 0..16usize {
                        if emu.bus.vdc_vram_word((base + w) as u16) != 0 {
                            all_zero = false;
                            break;
                        }
                    }
                    if all_zero {
                        println!("  '{}' tile {:03X}: EMPTY", ch, tid);
                    } else {
                        // Show plane 0 pattern
                        print!("  '{}' tile {:03X} p0: ", ch, tid);
                        for row in 0..8usize {
                            let w = emu.bus.vdc_vram_word((base + row) as u16);
                            let p0 = (w & 0xFF) as u8;
                            for bit in (0..8).rev() {
                                if (p0 >> bit) & 1 != 0 {
                                    print!("#");
                                } else {
                                    print!(".");
                                }
                            }
                            print!("|");
                        }
                        println!();
                    }
                }
                // Also check total VRAM writes
                println!(
                    "  VRAM high writes so far: {}",
                    emu.bus.vdc_vram_data_high_writes()
                );
                cp_idx += 1;
            }
        }
    }

    // Now check the ROM font data. We found '0' at ROM 0x02E400 (stride-2 interleaved).
    // Let's find 'H' in ROM
    let rom_data = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;

    // Expected 'H' patterns - common 8x8 font 'H':
    // C6 C6 FE FE C6 C6 C6 00 or similar
    println!("\n=== Searching for 'H' font in ROM ===");

    // Let's look at what's at ROM offset 0x02E400 + 24*32 = 0x02E700 (where 'H' should be if font starts at '0')
    let h_offset = 0x02E400 + (0x48 - 0x30) * 32;
    println!("Expected 'H' ROM offset: {:06X}", h_offset);
    if h_offset + 32 <= rom_data.len() {
        print!("ROM data: ");
        for i in 0..32 {
            print!("{:02X} ", rom_data[h_offset + i]);
        }
        println!();
        // Decode as tile (stride-2 interleaved like '0')
        print!("  Plane 0: ");
        for row in 0..8 {
            let b = rom_data[h_offset + row * 2];
            for bit in (0..8).rev() {
                if (b >> bit) & 1 != 0 {
                    print!("#");
                } else {
                    print!(".");
                }
            }
            print!("|");
        }
        println!();
    }

    // Actually, let me check what's at the font base more carefully
    // If '0' = tile 0x130 is at ROM 0x02E400, and the tile mapping is tile_id = 0x100 + ascii,
    // then tile 0x100 (ascii 0x00) would be at ROM 0x02E400 - 0x30*32 = 0x02E400 - 0x600 = 0x02DE00
    let font_base = 0x02E400usize - 0x30 * 32;
    println!("\nFont base (tile 0x100): ROM {:06X}", font_base);

    // Check 'H' = tile 0x148 = ASCII 0x48
    let h_from_base = font_base + 0x48 * 32;
    println!("'H' (tile 0x148): ROM {:06X}", h_from_base);
    if h_from_base + 32 <= rom_data.len() {
        print!("ROM data: ");
        for i in 0..32 {
            print!("{:02X} ", rom_data[h_from_base + i]);
        }
        println!();
        print!("  Plane 0 (even bytes): ");
        for row in 0..8 {
            let b = rom_data[h_from_base + row * 2];
            for bit in (0..8).rev() {
                if (b >> bit) & 1 != 0 {
                    print!("#");
                } else {
                    print!(".");
                }
            }
            print!("|");
        }
        println!();
    }

    // Also check ' ' (space) = tile 0x140 = ASCII 0x40
    // Wait, space is ASCII 0x20, not 0x40. The tile mapping in BAT row 24 shows:
    // 0x140 for spaces between words. 0x140 = 0x100 + 0x40. ASCII 0x40 = '@'.
    // But they're using it as space. Hmm, that means the mapping might not be simple ASCII+0x100.

    // Let me check: in the BAT, row 26 has "( ) 1987 HUDSON SOFT"
    // 13D = 0x100 + 0x3D = '=' but should be '(' ...
    // Actually 0x3D = '=' in ASCII. '(' = 0x28.
    // 13D doesn't match '(' (0x28) so the tile mapping is NOT ASCII + 0x100.

    // Let me look at row 26 more carefully:
    // 13Dp5 140p5 131p5 139p5 138p5 137p5 140p5 148p5 155p5 144p5 153p5 14Fp5 14Ep5 140p5 153p5 14Fp5 146p5 154p5
    // Expected text: "(C) 1987 HUDSON SOFT" or "© 1987 HUDSON SOFT"
    // 13D = ©/( , 140 = space, 131=1, 139=9, 138=8, 137=7
    // So: 131='1', 137='7', 138='8', 139='9'
    // 131 = 0x100 + 0x31. ASCII 0x31 = '1' ✓
    // 137 = 0x100 + 0x37. ASCII 0x37 = '7' ✓
    // 138 = 0x100 + 0x38. ASCII 0x38 = '8' ✓
    // 139 = 0x100 + 0x39. ASCII 0x39 = '9' ✓
    // OK so numbers ARE ASCII + 0x100.
    // 13D = 0x100 + 0x3D = '=' but should be '(' or '©'
    // Maybe it's a custom character, not standard ASCII

    // Continue: 148=H, 155=U, 144=D, 153=S, 14F=O, 14E=N
    // H=0x48✓, U=0x55✓, D=0x44✓, S=0x53✓, O=0x4F✓, N=0x4E✓
    // So uppercase letters ARE ASCII + 0x100.

    // 140 = 0x100 + 0x40 = '@'. But used as space.
    // In some Japanese game fonts, '@' position is used for space because 0x20 might be blank.

    println!("\n=== ROM font check for specific characters ===");
    for &(ch, ascii) in &[
        ('0', 0x30u8),
        ('1', 0x31),
        ('H', 0x48),
        ('I', 0x49),
        ('S', 0x53),
        ('P', 0x50),
        ('U', 0x55),
        ('N', 0x4E),
    ] {
        let offset = font_base + (ascii as usize) * 32;
        if offset + 32 <= rom_data.len() {
            print!("  '{}' (ROM {:06X}): ", ch, offset);
            for row in 0..8 {
                let b = rom_data[offset + row * 2];
                for bit in (0..8).rev() {
                    if (b >> bit) & 1 != 0 {
                        print!("#");
                    } else {
                        print!(".");
                    }
                }
                print!("|");
            }
            println!();
        }
    }

    Ok(())
}
