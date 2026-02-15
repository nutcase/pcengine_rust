# Complete HuC6280 PSG Audio Path
This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.
This document follows `.agent/PLANS.md` from the repository root and is maintained in accordance with its requirements.

## Purpose / Big Picture
After this change, HuCard programs can drive audible PSG output through the existing sample pipeline with hardware-like channel behavior: wave channels, DDA direct output, per-channel balance, master balance, and noise channels. The result is observable by running the SDL audio example with a ROM and hearing non-trivial sound instead of a flat or mostly silent signal.

## Progress
- [x] (2026-02-15 10:58Z) Audited current PSG/audio path and test coverage in `src/bus.rs`, `src/emulator.rs`, and `examples/audio_sdl.rs`.
- [x] (2026-02-15 11:24Z) Implemented HuC6280-style PSG register model and channel state machine in `src/bus.rs` (wave, DDA, noise, LFO, balance, timer IRQ2 compatibility).
- [x] (2026-02-15 11:27Z) Added/adjusted unit tests for DDA, noise, balance, wave-write gating, DDA index reset, and frequency divider behavior.
- [x] (2026-02-15 11:02Z) Updated `examples/audio_sdl.rs` to load a ROM (`.pce` via `load_hucard`) and stream FIFO audio samples.
- [x] (2026-02-15 11:30Z) Switched bus audio sample scheduling to ratio-based accumulation to avoid long-run timing drift from integer truncation.
- [x] (2026-02-15 11:33Z) Ran formatting and tests (`cargo fmt`, `cargo test --lib`, `cargo check --example audio_sdl --features audio-sdl`) and captured pass results.

## Surprises & Discoveries
- Observation: The current PSG model is register-index based with auto-increment and a synthetic timer (`0x18..0x1A`) that also feeds IRQ2 tests.
  Evidence: `src/bus.rs` lines around `PSG_REG_TIMER_LO/HI/CTRL`, `psg_irq2_triggers_when_enabled` test.
- Observation: SDL audio example currently boots an empty emulator without loading ROM content.
  Evidence: `examples/audio_sdl.rs` initializes `Emulator::new(); emu.reset();` without `load_hucard`.
- Observation: Hardware PSG wave RAM writes are ignored while channel enable or DDA is set, and DDA clear resets waveform index.
  Evidence: `docs/pcetech.txt` section "10.) Programmable Sound Generator" (lines around 1692-1700); new tests `psg_wave_writes_ignored_while_channel_enabled`, `psg_clearing_dda_resets_wave_write_index`.
- Observation: Using `MASTER_CLOCK_HZ / AUDIO_SAMPLE_RATE` integer truncation causes steady sample-rate drift over long runs.
  Evidence: old `enqueue_audio_samples` used fixed 162-cycle step; now replaced with ratio accumulator `phi_cycles * AUDIO_SAMPLE_RATE / MASTER_CLOCK_HZ`.

## Decision Log
- Decision: Keep legacy PSG timer registers (`0x18..0x1A`) for IRQ2 compatibility while adding hardware-like audio register behavior for `0x00..0x09`.
  Rationale: Minimizes regression risk while delivering user-visible audio completion.
  Date/Author: 2026-02-15 / Codex
- Decision: Model frequency register as divider semantics (`0x001` highest pitch, `0x000` lowest) instead of direct increment.
  Rationale: Matches HuC6280 PSG behavior and fixes inverted pitch relationship.
  Date/Author: 2026-02-15 / Codex
- Decision: Keep mono output API while preserving left/right balance in internal mix by averaging channels.
  Rationale: Avoids public API churn (`Vec<i16>`) while still respecting per-channel/master balance behavior.
  Date/Author: 2026-02-15 / Codex

## Outcomes & Retrospective
PSG audio path now produces ROM-driven audible output with significantly improved hardware semantics: wave channels obey write gating rules, DDA-to-wave transitions reset index correctly, frequency divider behavior is no longer inverted, noise and balance are exercised by tests, and sample scheduling drift is reduced by ratio accumulation. The SDL audio example now boots a ROM and plays FIFO audio correctly.

Remaining work outside this plan is full host front-end integration (single executable with video+audio+input) and cycle-perfect PSG/LFO nuances.

## Context and Orientation
Audio samples are generated in `src/bus.rs` (`Bus::enqueue_audio_samples`) and pulled into `src/emulator.rs` (`Emulator::take_audio_samples`). PSG register writes are routed from hardware-page I/O (`0x0800..0x0BFF` mirrored) into `Psg::write_address` and `Psg::write_data`. The current PSG model stores generic registers and emits simplistic waveform-based mono output. The planned change keeps the existing mono sample interface (`Vec<i16>`) and improves signal generation semantics.

## Plan of Work
Edit `src/bus.rs` in the `Psg` section to model channel selection and per-channel state explicitly. Add fields for per-channel frequency, control flags (key-on, DDA), channel balance, waveform RAM write/read index, noise state, and a simple LFO modulator path. Preserve the existing timer IRQ bits and behavior used by tests.

Update `Psg::write_data` to decode `0x00..0x09` as HuC6280-style audio registers and retain high-index waveform RAM compatibility for existing tests. Update `generate_sample` and helpers so mixing uses channel and master balance. Add clipping-safe conversion to `i16`.

Add unit tests in `src/bus.rs` to verify observable audio behavior: DDA value appears in output when keyed on, noise channels produce varying samples, and changing balance changes output amplitude.

Update `examples/audio_sdl.rs` to accept a ROM path (`.pce`) and run the actual game audio path.

## Concrete Steps
From `/Users/takamatsu/dev/pce`:
  1. Edit `src/bus.rs` PSG constants/structs/methods.
  2. Add PSG behavior tests in `src/bus.rs` test module.
  3. Edit `examples/audio_sdl.rs` to load ROM path.
  4. Run `cargo fmt`.
  5. Run `cargo test --lib`.
  6. Run `cargo run --example audio_sdl --features audio-sdl -- roms/Kato-chan\ \&\ Ken-chan\ \(Japan\).pce` for manual validation.

## Validation and Acceptance
Acceptance criteria:
- `cargo test --lib` passes with new PSG tests.
- `examples/audio_sdl` can load a HuCard and produce continuous non-zero sample output audibly.
- Existing IRQ2 PSG timer test remains passing.

## Idempotence and Recovery
Edits are additive and local to PSG/audio paths. If a regression appears, restore only the `Psg` section and new tests, keeping unrelated rendering/CPU changes untouched.

## Artifacts and Notes
  cargo test --lib -q
  running 175 tests
  ...
  test result: ok. 175 passed; 0 failed

  cargo check --example audio_sdl --features audio-sdl -q
  (no errors)

## Interfaces and Dependencies
- Keep `Bus::take_audio_samples(&mut self) -> Vec<i16>` unchanged.
- Keep `Emulator::take_audio_samples(&mut self) -> Option<Vec<i16>>` unchanged.
- Keep PSG port mapping (`0x0800` address, `0x0801` data) unchanged.
- Add internal helpers inside `impl Psg` only; do not change public `Bus` API.

Revision Note (2026-02-15 / Codex): Initial plan created to guide implementation and verification for PSG audio completion.
Revision Note (2026-02-15 / Codex): Updated plan after implementation to reflect completed milestones, added PSG hardware behavior fixes, and recorded validation output.
