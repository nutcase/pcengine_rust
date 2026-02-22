# Repository Guidelines

## Project Structure & Module Organization
Rust sources live in `src/`: `lib.rs` exposes common types, `main.rs` drives the CLI, and `cpu.rs`, `bus.rs`, `emulator.rs` house the HuC6280 core, memory/IO fabric, and run loop. Inline `#[cfg(test)]` modules stay beside the code they exercise. Developer harnesses live in `examples/` (`trace_boot`, `dump_frame`, `audio_sdl`), and work-in-progress HuCards stay in `roms/` (leave licensed images untracked). Keep `README.md` and `TODO.md` aligned with the project roadmap whenever features shift.

## Build, Test, and Development Commands
- `cargo fmt` — normalize formatting before commits.
- `cargo clippy --all-targets --all-features -D warnings` — enforce lint cleanliness across binaries and examples.
- `cargo test` — run CPU, bus, and emulator suites; required for every change.
- `cargo run -- roms/<image>.pce` — smoke-test HuCard boot paths.
- `cargo run --example trace_boot roms/<image>.pce` — trace reset and verify hardware flags.
- `cargo run --example dump_frame roms/<image>.pce` — produce `frame.ppm` when iterating on VDC output.
- `cargo run --example audio_sdl roms/<image>.pce --features audio-sdl` — audit PSG audio (needs local SDL2 libs).

## Coding Style & Naming Conventions
Follow idiomatic Rust: four-space indentation, `snake_case` for items, `UpperCamelCase` for types, and screaming snake for constants or register masks. Keep cross-cutting helpers private to their module, prefer typed wrappers over raw integers for cycles or addresses, and annotate tricky hardware behavior with short comments plus reference links. Always run `cargo fmt` and `cargo clippy` before pushing to guarantee deterministic diffs and lint-free CI.

## Testing Guidelines
Pair new logic with focused unit tests near the implementation. For behaviors spanning modules—such as VDC status clearing or IRQ acknowledgement—extend the `examples/` harnesses or add integration tests under `tests/`. Use only small diagnostic ROMs in `roms/`, reference larger assets in documentation instead, and record any manual verification steps in `TODO.md`. When producing artifacts (frames, audio dumps), attach them to review threads rather than committing binaries.

## Commit & Pull Request Guidelines
With no public history yet, adopt imperative present-tense commit titles (`Implement VDC DMA busy flag`). Each commit must compile and pass tests independently. PRs should outline intent, list verification commands (`cargo test`, example runs), link relevant TODO milestones, and mention external sources consulted. Surface visual or audio changes with before/after attachments, and update `README.md`/`TODO.md` for any milestones completed or re-scoped.

## Hardware References & Research Notes
Favor primary HuC6280/6270/6260 manuals, cite sources inline when porting behavior, and keep only redistributable documents under version control.

docs/65C02_Opcodes.txt
docs/pcetech.txt
docs/doc_links.txt
docs/vdcdox.txt
docs/HuC6270 - CMOS Video Display Controller Manual.pdf

# ExecPlans
When writing complex features or significant refactors, use an ExecPlan (as described in .agent/PLANS.md) from design to implementation.
