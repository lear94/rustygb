//! Steady-state cost of emulating one frame of a synthetic ROM.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rusty_gb::GameBoy;

const ROM_LEN: usize = 0x8000;
const ENTRY_POINT: usize = 0x0100;
const SAMPLE_RATE_HZ: u32 = 48_000;

/// LD HL,C000; loop: LD A,(HL+); ADD A,B; LD (HL),A; PUSH AF; POP BC; RES 5,H; JR loop.
const PROGRAM: [u8; 12] = [
    0x21, 0x00, 0xC0, 0x2A, 0x80, 0x77, 0xF5, 0xC1, 0xCB, 0xAC, 0x18, 0xF7,
];

fn bench_run_frame(c: &mut Criterion) {
    let mut rom = vec![0; ROM_LEN];
    rom[ENTRY_POINT..ENTRY_POINT + PROGRAM.len()].copy_from_slice(&PROGRAM);
    let mut gb = GameBoy::from_rom(rom, SAMPLE_RATE_HZ).expect("synthetic ROM is valid");
    c.bench_function("run_frame", |b| {
        b.iter(|| {
            gb.run_frame();
            black_box(gb.frame().first());
        });
    });
}

criterion_group!(benches, bench_run_frame);
criterion_main!(benches);
