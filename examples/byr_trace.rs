use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let mut frames = 0;
    let mut prev_byr = 0xFFFFu16;
    println!("BYR values over first 600 frames:");
    while frames < 600 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
            let byr = emu.bus.vdc_register(0x08).unwrap_or(0);
            if byr != prev_byr || frames <= 10 || frames % 10 == 0 {
                let mwr = emu.bus.vdc_register(0x09).unwrap_or(0);
                let (w, h) = emu.bus.vdc_map_dimensions();
                let map_pixel_h = h * 8;
                // With flat addressing, title rows 4-9 start at sample_y=32
                // Title visible when: sample_y=32 < byr + 224 AND sample_y=32 >= byr
                // i.e., byr <= 32 (for title top to be visible from active_row=0)
                let title_y = 32; // flat row 4 * 8
                let title_visible = if byr as usize <= title_y {
                    "VISIBLE"
                } else if title_y + map_pixel_h > byr as usize + 224 {
                    "wrapped"
                } else {
                    "not visible"
                };
                println!(
                    "  Frame {:3}: BYR={:3} ({:#06X}) MWR={}x{} title_top={}",
                    frames, byr, byr, w, h, title_visible
                );
                prev_byr = byr;
            }
        }
    }

    Ok(())
}
