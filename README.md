# PC Engine Emulator (Rust)

This project implements the core scaffolding for a PC Engine (HuC6280) emulator in Rust. It focuses on building confidence in the CPU pipeline, banking model, and developer ergonomics for rapid experimentation.

## Features
- Bank-aware 64 KiB memory window with configurable 8 KiB pages backing RAM or ROM data.
- HuC6280 CPU core covering loads/stores, arithmetic/logic, branches, block transfers (`TII/TIN/TDD/TIA/TAI`), memory page register moves (`TMA/TAM`), register swaps (`SAX/SAY/SXY`), stack/flag control (`CLA/CLX/CLY`, `SET`, `CSH/CSL`, `BSR`), and the ST0/1/2 VDC immediates.
- MPR bank switching implemented at `$FF80`-`$FF87`, with a simple I/O page backing for `$FF`-mapped segments (sufficient for early device stubbing).
- Interrupt scaffold including `WAI`/`RTI`, IRQ/NMI request handling, and stack push/pull semantics.
- Early VDC renderer that draws background tile maps and SATB-driven sprites (with basic priority handling) into a software framebuffer.
- HuC6270 DMA paths (VRAM↔VRAM, SATB auto-transfer, CRAM palette DMA) with busy/status flag updates that match the DS/DV handshake BIOS routines expect.
- Execution loop that runs until a `BRK` instruction or a cycle budget is hit.
- Unit tests exercising CPU arithmetic, branching, bank switching, and the top-level emulator workflow.

## Usage
```
cargo run -- path/to/program.bin
```

For HuCard images:
```
cargo run -- path/to/card.pce
```

Optional flags:
```
cargo run -- path/to/card.pce --load-backup saves/card.sav --save-backup saves/card.sav --frame-limit 2
```

`--load-backup` preloads HuCard backup RAM before reset, `--save-backup` writes the current RAM image after execution (no effect for raw `.bin` programs), and `--frame-limit <n>` runs only until `n` frames have been produced. When omitted, HuCards automatically load/save `path/to/card.sav` alongside the ROM if the file exists. Programs are loaded at `$C000` by default. Break out of execution with a `BRK` (opcode `0x00`).

HuCard images (`.pce`) have their optional 512-byte headers removed automatically, and the upper four MPR slots (4–7) are mapped to the highest ROM banks so the reset vector and startup code are available immediately; if those banks are blank, the loader falls back to the default sequential bank layout.

To experiment with ROM banking in code, call `Bus::load_rom_image` followed by `Bus::map_bank_to_rom`/`map_bank_to_ram` before issuing `emulator.reset()`.

### Examples

- `cargo run --example trace_boot roms/sample_game.pce` — trace CPU execution and print bus state.
- `cargo run --example dump_frame roms/sample_game.pce` — dump the next rendered frame into `frame.ppm` for quick inspection.
- `cargo run --example video_sdl --features video-sdl roms/sample_game.pce` — open an SDL window and stream frames in real time.
  Controls: arrows move the D-pad, `Z`/`X` map to buttons I/II, `Enter` is Select, and `Space` is Run.
  Sprite priority handling and DMA-driven palette uploads mirror the HuC6270 behaviour so BIOS startup code can rely on SATB/CRAM transfers completing automatically.

## Next Steps
- Fill out the remaining HuC6280 instructions (block moves, bit manipulation, interrupts) and timing nuances.
- Model the VDC, PSG, timers, and I/O registers via pluggable bus devices.
- Add ROM loaders for `.pce` images with header parsing and automatic bank setup.
- Integrate rendering and audio back-ends for a complete user-facing emulator.

Refer to `TODO.md` for the live roadmap toward booting `roms/Kato-chan & Ken-chan (Japan).pce`, including CPU work, device modelling, and front-end integration milestones.
