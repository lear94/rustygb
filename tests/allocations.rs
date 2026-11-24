//! The steady state never allocates.
#![allow(
    clippy::expect_used,
    reason = "test fixtures: clippy only exempts #[test] functions"
)]

use rusty_gb::{Button, GameBoy};

/// Allocations allowed per measured run.
const ALLOCATION_BUDGET: u64 = 0;
const FRAMES: usize = 120;
/// Every layer, sprites and channels on.
const BUSY_SCENE: [(u16, u8); 11] = [
    (0xFF25, 0xFF),
    (0xFF12, 0xF0),
    (0xFF14, 0x87),
    (0xFF17, 0xF0),
    (0xFF19, 0x87),
    (0xFF1A, 0x80),
    (0xFF1E, 0x87),
    (0xFF21, 0xF0),
    (0xFF23, 0x80),
    (0xFF4B, 0x57),
    (0xFF40, 0xF7),
];

fn busy_game() -> GameBoy {
    let mut gb = GameBoy::from_rom(vec![0; 0x8000], 48_000).expect("blank ROM is valid");
    gb.bus().write_byte(0xFF40, 0);
    for offset in 0..160 {
        gb.bus().write_byte(0xFE00 + offset, 16);
    }
    for (addr, val) in BUSY_SCENE {
        gb.bus().write_byte(addr, val);
    }
    gb
}

#[test]
fn run_frame_steady_state_does_not_allocate() {
    let mut gb = busy_game();
    let mut samples = [0.0; 1024];
    let measured = allocation_counter::measure(|| {
        for _ in 0..FRAMES {
            gb.set_button(Button::A, true);
            gb.run_frame();
            gb.drain_audio(&mut samples);
        }
    });
    assert_eq!(measured.count_total, ALLOCATION_BUDGET);
}

#[test]
fn run_frame_undrained_audio_does_not_allocate() {
    let mut gb = busy_game();
    let measured = allocation_counter::measure(|| (0..FRAMES).for_each(|_| gb.run_frame()));
    assert_eq!(measured.count_total, ALLOCATION_BUDGET);
}
