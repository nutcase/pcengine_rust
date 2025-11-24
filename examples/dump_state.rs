use pce::{bus::Bus, emulator::Emulator};
use std::{env, error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(feature = "trace_hw_writes")]
    eprintln!("trace_hw_writes feature active (dump_state)");
    let mut args = env::args().skip(1);
    let rom_path = args
        .next()
        .ok_or("usage: dump_state <rom.[bin|pce]> [frames] [cycles]")?;
    let frame_target: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(10);
    let cycle_budget: u64 = args
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000_000);

    let rom = fs::read(&rom_path)?;
    let is_pce = Path::new(&rom_path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);

    let mut emulator = Emulator::new();
    if is_pce {
        emulator.load_hucard(&rom)?;
    } else {
        emulator.load_program(0xC000, &rom);
    }
    let initial_mpr = emulator.bus.mpr_array();
    emulator.reset();

    let mut frames = 0usize;
    while frames < frame_target && emulator.cycles() < cycle_budget {
        let cycles = emulator.tick() as u64;
        if cycles == 0 && !emulator.cpu.is_waiting() {
            break;
        }
        if emulator.take_frame().is_some() {
            frames += 1;
        }
    }

    let (nonzero, unique) = vram_stats(&mut emulator.bus);

    println!("frames collected: {frames}");
    println!("cycles executed: {}", emulator.cycles());
    println!(
        "vram words: {} non-zero: {} unique(non-zero): {}",
        VRAM_WORDS, nonzero, unique
    );
    println!(
        "cpu pc: {:#06X} sp: {:#04X}",
        emulator.cpu.pc, emulator.cpu.sp
    );
    println!(
        "vdc control: {:04X} brightness: {:X}",
        emulator.bus.vdc_register(0x05).unwrap_or(0),
        emulator.bus.vce_last_control_high() >> 4
    );
    println!(
        "mpr (boot): {}",
        initial_mpr
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{}:{:02X}", i, v))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mpr = emulator.bus.mpr_array();
    println!(
        "mpr: {}",
        mpr.iter()
            .enumerate()
            .map(|(i, v)| format!("{}:{:02X}", i, v))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "vdc control writes: {} last value: {:04X}",
        emulator.bus.vdc_control_write_count(),
        emulator.bus.vdc_last_control()
    );
    println!(
        "vdc dcr writes: {} last value: {:02X}",
        emulator.bus.vdc_dcr_write_count(),
        emulator.bus.vdc_last_dcr_value()
    );
    for idx in 0x04..=0x0B {
        if let Some(value) = emulator.bus.vdc_register(idx) {
            println!("vdc R{idx:02X} = {:04X}", value);
        }
    }
    for (label, index) in [
        ("R04", 0x04usize),
        ("R05", 0x05usize),
        ("R0C", 0x0Cusize),
        ("R0F", 0x0Fusize),
        ("R10", 0x10usize),
        ("R11", 0x11usize),
        ("R12", 0x12usize),
        ("R13", 0x13usize),
    ] {
        println!(
            "vdc {label} writes: {} selects: {}",
            emulator.bus.vdc_register_write_count(index),
            emulator.bus.vdc_register_select_count(index)
        );
    }
    println!(
        "vdc R05 data writes: low {} high {}",
        emulator.bus.vdc_r05_low_writes(),
        emulator.bus.vdc_r05_high_writes()
    );
    println!(
        "vdc R05 last low byte: {:02X}",
        emulator.bus.vdc_last_r05_low()
    );
    println!("top IO writes:");
    for (addr, count) in emulator.bus.io_write_hist_top(10) {
        println!("  {:04X}: {}", addr, count);
    }
    // Summaries of which registers were touched during the run.
    print!("vdc select counts:");
    for (i, cnt) in emulator.bus.vdc_register_select_counts().iter().enumerate() {
        if *cnt > 0 {
            print!(" R{:02X}={}", i, cnt);
        }
    }
    println!();

    print!("vdc write counts :");
    for (i, cnt) in emulator.bus.vdc_register_write_counts().iter().enumerate() {
        if *cnt > 0 {
            print!(" R{:02X}={}", i, cnt);
        }
    }
    println!();
    let alias_counts = emulator.bus.vdc_alias_write_counts();
    let mut alias_stats: Vec<(usize, u64)> = alias_counts
        .iter()
        .enumerate()
        .filter_map(|(slot, &count)| if count > 0 { Some((slot, count)) } else { None })
        .collect();
    if !alias_stats.is_empty() {
        alias_stats.sort_by(|a, b| b.1.cmp(&a.1));
        print!("vdc alias writes:");
        for (slot, count) in alias_stats.iter().take(8) {
            print!(" {:02X}:{count}", slot);
        }
        println!();
    }
    println!(
        "mawr {:04X} marr {:04X}",
        emulator.bus.vdc_register(0x00).unwrap_or(0),
        emulator.bus.vdc_register(0x01).unwrap_or(0)
    );
    println!(
        "cram dma last src {:04X} len {:04X}",
        emulator.bus.vdc_cram_last_source(),
        emulator.bus.vdc_cram_last_length()
    );
    println!(
        "vram dma count {} last src {:04X} dst {:04X} len {:04X}",
        emulator.bus.vdc_vram_dma_count(),
        emulator.bus.vdc_vram_last_source(),
        emulator.bus.vdc_vram_last_destination(),
        emulator.bus.vdc_vram_last_length()
    );
    println!(
        "satb pending {} source {:04X}",
        emulator.bus.vdc_satb_pending(),
        emulator.bus.vdc_satb_source()
    );
    println!(
        "zp[00]={:02X} zp[01]={:02X} zp[20]={:02X} zp[21]={:02X} zp[22]={:02X} zp[23]={:02X}",
        emulator.bus.read_zero_page(0),
        emulator.bus.read_zero_page(1),
        emulator.bus.read_zero_page(0x20),
        emulator.bus.read_zero_page(0x21),
        emulator.bus.read_zero_page(0x22),
        emulator.bus.read_zero_page(0x23)
    );
    println!(
        "dma ctrl {:04X} src {:04X} dst {:04X} len {:04X}",
        emulator.bus.vdc_register(0x0F).unwrap_or(0),
        emulator.bus.vdc_register(0x10).unwrap_or(0),
        emulator.bus.vdc_register(0x11).unwrap_or(0),
        emulator.bus.vdc_register(0x12).unwrap_or(0)
    );

    let cram_src = emulator.bus.vdc_cram_last_source();
    if cram_src != 0 {
        print!("VRAM[{cram_src:04X}] ->");
        for offset in 0..8u16 {
            let addr = cram_src.wrapping_add(offset);
            let word = read_vram_word(&mut emulator.bus, addr);
            print!(" {word:04X}");
        }
        println!();
    }
    let palette_vram_base = 0x0500u16;
    print!("VRAM[{palette_vram_base:04X}] ->");
    for offset in 0..8u16 {
        let addr = palette_vram_base.wrapping_add(offset);
        let word = read_vram_word(&mut emulator.bus, addr);
        print!(" {word:04X}");
    }
    println!();

    let palette_nonzero = palette_stats(&mut emulator.bus);
    println!("palette entries non-zero: {palette_nonzero}");
    for idx in 0..16usize {
        let value = read_palette_word(&mut emulator.bus, idx as u16);
        println!("palette[{idx:02X}] = {value:04X}");
    }

    for base in [
        0x0000u16, 0x0400, 0x0800, 0x0C00, 0x1000, 0x1400, 0x1800, 0x1C00, 0x2000, 0x2400, 0x2800,
        0x2C00, 0x3000, 0x3800, 0x6000,
    ] {
        print!("VRAM[{base:04X}]:");
        for offset in 0..16u16 {
            let addr = base.wrapping_add(offset);
            let word = read_vram_word(&mut emulator.bus, addr);
            print!(" {word:04X}");
        }
        println!();
    }

    println!(
        "VCE writes observed: {} (data writes: {})",
        emulator.bus.vce_write_count(),
        emulator.bus.vce_data_write_count()
    );
    println!(
        "VCE control writes: {}",
        emulator.bus.vce_control_write_count()
    );
    println!("VCE port hits: {}", emulator.bus.vce_port_hit_count());
    println!("CRAM DMA scheduled: {}", emulator.bus.cram_dma_count());
    println!(
        "Last VCE port addr: {:04X}",
        emulator.bus.vce_last_port_addr()
    );
    println!(
        "Last VCE control high: {:02X} (max {:02X})",
        emulator.bus.vce_last_control_high(),
        emulator.bus.vce_last_control_high_max()
    );

    Ok(())
}

const VRAM_WORDS: usize = 0x8000;

fn read_vram_word(bus: &mut Bus, addr: u16) -> u16 {
    bus.write_st_port(0, 0x01);
    bus.write_st_port(1, (addr & 0xFF) as u8);
    bus.write_st_port(2, ((addr >> 8) & 0x7F) as u8);
    bus.write_st_port(0, 0x02);
    let lo = bus.read_st_port(1);
    let hi = bus.read_st_port(2);
    u16::from_le_bytes([lo, hi])
}

fn vram_stats(bus: &mut Bus) -> (usize, usize) {
    use std::collections::HashSet;
    let mut nonzero = 0usize;
    let mut set = HashSet::new();
    for addr in 0..VRAM_WORDS {
        let value = read_vram_word(bus, addr as u16);
        if value != 0 {
            nonzero += 1;
            set.insert(value);
        }
    }
    (nonzero, set.len())
}

const VCE_CONTROL_LOW: u16 = 0x0400;
const VCE_CONTROL_HIGH: u16 = 0x0401;
const VCE_DATA_LOW: u16 = 0x0402;
const VCE_DATA_HIGH: u16 = 0x0403;

fn read_palette_word(bus: &mut Bus, index: u16) -> u16 {
    let control_low = (index & 0xFF) as u8;
    let control_high = ((index >> 8) & 0x01) as u8;
    bus.write(VCE_CONTROL_LOW, control_low);
    bus.write(VCE_CONTROL_HIGH, control_high);
    let lo = bus.read(VCE_DATA_LOW);
    let hi = bus.read(VCE_DATA_HIGH);
    u16::from_le_bytes([lo, hi])
}

fn palette_stats(bus: &mut Bus) -> usize {
    let mut count = 0usize;
    for idx in 0..0x200u16 {
        if read_palette_word(bus, idx) != 0 {
            count += 1;
        }
    }
    count
}
