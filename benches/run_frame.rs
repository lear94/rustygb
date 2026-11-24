//! Steady-state cost of emulating one frame of a synthetic ROM.

#![allow(
    missing_docs,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "harness code: the macro generates undocumented items and a bad ROM must abort"
)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rusty_gb::GameBoy;

const ROM_LEN: usize = 0x8000;
const ENTRY_POINT: usize = 0x0100;
const SAMPLE_RATE_HZ: u32 = 48_000;

/// LD HL,C000; loop: LD A,(HL+); ADD A,B; LD (HL),A; PUSH AF; POP BC; RES 5,H; JR loop.
const PROGRAM: [u8; 12] = [
    0x21, 0x00, 0xC0, 0x2A, 0x80, 0x77, 0xF5, 0xC1, 0xCB, 0xAC, 0x18, 0xF7,
];
const OAM_BASE: u16 = 0xFE00;
/// Registers of a busy scene: every layer, every channel, the fastest timer.
const SCENE: [(u16, u8); 21] = [
    (0xFF25, 0xFF),
    (0xFF12, 0xF0),
    (0xFF14, 0x87),
    (0xFF17, 0xF0),
    (0xFF19, 0x87),
    (0xFF1A, 0x80),
    (0xFF1C, 0x20),
    (0xFF1D, 0xFF),
    (0xFF1E, 0x87),
    (0xFF21, 0xF0),
    (0xFF22, 0x00),
    (0xFF23, 0x80),
    (0xFF07, 0x05),
    (0xFF47, 0xE4),
    (0xFF48, 0xE4),
    (0xFF4A, 0x00),
    (0xFF4B, 0x57),
    (0xFF42, 0x03),
    (0xFF43, 0x05),
    (0x8010, 0xFF),
    (0xFF40, 0xF7),
];

fn game() -> GameBoy {
    let mut rom = vec![0; ROM_LEN];
    rom[ENTRY_POINT..ENTRY_POINT + PROGRAM.len()].copy_from_slice(&PROGRAM);
    GameBoy::from_rom(rom, SAMPLE_RATE_HZ).expect("synthetic ROM is valid")
}

fn busy_game() -> GameBoy {
    let mut gb = game();
    let bus = gb.bus();
    bus.write_byte(0xFF40, 0);
    for sprite in 0..40u8 {
        let entry = [16 + sprite / 10 * 16, 8 + sprite % 10 * 16, 1, 0];
        for (offset, val) in (u16::from(sprite) * 4..).zip(entry) {
            bus.write_byte(OAM_BASE + offset, val);
        }
    }
    for (addr, val) in SCENE {
        bus.write_byte(addr, val);
    }
    gb
}

type Build = fn() -> GameBoy;

fn bench_run_frame(c: &mut Criterion) {
    for (name, build) in [("run_frame", game as Build), ("run_frame_busy", busy_game)] {
        let mut gb = build();
        c.bench_function(name, |b| {
            b.iter(|| {
                gb.run_frame();
                black_box(gb.frame().first());
            });
        });
    }
}

criterion_group!(benches, bench_run_frame);
criterion_main!(benches);
