//! Arbitrary cartridges, saves, code and bus traffic must never panic.
//!
//! The seed is fixed for reproducibility; set `PROPTEST_CASES` for a longer campaign.

#![allow(
    clippy::indexing_slicing,
    reason = "test fixtures: clippy only exempts #[test] functions"
)]

use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};
use rusty_gb::{Button, GameBoy, SCREEN_HEIGHT, SCREEN_WIDTH};

const MAPPERS: [u8; 24] = [
    0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x08, 0x09, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x19, 0x1A, 0x1B,
    0x1C, 0x1D, 0x1E, 0x04, 0x20, 0x22, 0xFC, 0xFF,
];
const BUTTONS: [Button; 8] = [
    Button::A,
    Button::B,
    Button::Select,
    Button::Start,
    Button::Right,
    Button::Left,
    Button::Up,
    Button::Down,
];
const HEADER_END: usize = 0x150;

/// ROM whose code is arbitrary and whose header is plausible often enough to load.
fn rom() -> impl Strategy<Value = Vec<u8>> {
    (
        vec(any::<u8>(), 0..0x1_4000),
        proptest::sample::select(MAPPERS.to_vec()),
        0u8..7,
    )
        .prop_map(|(mut rom, mapper, ram)| {
            if rom.len() >= HEADER_END {
                rom[0x147] = mapper;
                rom[0x149] = ram;
            }
            rom
        })
}

fn save() -> impl Strategy<Value = Option<Vec<u8>>> {
    let lens = [
        0, 1, 44, 48, 512, 560, 0x800, 0x2000, 0x2030, 0x8000, 0x8030, 0x1_0000,
    ];
    proptest::option::of(
        proptest::sample::select(lens.to_vec()).prop_flat_map(|len| vec(any::<u8>(), len)),
    )
}

fn config(cases: u32) -> Config {
    let requested = std::env::var("PROPTEST_CASES").ok();
    Config {
        rng_seed: RngSeed::Fixed(0),
        failure_persistence: None,
        cases: requested
            .and_then(|text| text.parse().ok())
            .unwrap_or(cases),
        ..Config::default()
    }
}

proptest! {
    #![proptest_config(config(64))]

    #[test]
    fn run_frame_arbitrary_cartridge_never_panics(
        rom in rom(),
        save in save(),
        sample_rate in prop_oneof![Just(0), Just(1), 8_000u32..200_000, Just(u32::MAX)],
        now in any::<u64>(),
        presses in vec((0usize..8, any::<bool>()), 4),
    ) {
        let Ok(mut gb) = GameBoy::from_rom_with_save(rom, save, sample_rate) else {
            return Ok(());
        };
        gb.cartridge().sync_clock(now);
        let mut samples = vec![0.0; 4096];
        for (button, pressed) in presses {
            gb.set_button(BUTTONS[button], pressed);
            gb.run_frame();
            let count = gb.drain_audio(&mut samples);
            prop_assert!(samples[..count].iter().all(|sample| sample.abs() <= 1.0));
        }
        prop_assert_eq!(gb.frame().len(), SCREEN_WIDTH * SCREEN_HEIGHT * 4);
        let _ = gb.cartridge().save_image(now);
    }

    #[test]
    fn bus_arbitrary_traffic_never_panics(
        rom in rom(),
        traffic in vec((any::<u16>(), any::<u8>(), 0u8..40), 1..400),
    ) {
        let Ok(mut gb) = GameBoy::from_rom(rom, 48_000) else {
            return Ok(());
        };
        for (addr, val, ticks) in traffic {
            // Half of the traffic goes to the registers, where the state lives.
            let addr = if val & 1 == 0 { addr | 0xFF00 } else { addr };
            gb.bus().write_byte(addr, val);
            (0..ticks).for_each(|_| gb.bus().tick());
            let _ = gb.bus().read_byte(addr);
        }
        gb.run_frame();
    }

    #[test]
    fn save_image_reloads_identically(
        mapper in proptest::sample::select(vec![0x03u8, 0x06, 0x09, 0x10, 0x13, 0x1B]),
        ram in 1u8..6,
        writes in vec((0u8..16, 0xA000u16..0xC000, any::<u8>()), 1..100),
        now in 0u64..1 << 40,
    ) {
        let mut rom = vec![0; 0x8000];
        rom[0x147] = mapper;
        rom[0x149] = ram;
        let mut gb = GameBoy::from_rom(rom.clone(), 48_000).expect("supported cartridge");
        let cartridge = gb.cartridge();
        cartridge.write(0x0000, 0x0A);
        cartridge.write(0x6000, 1);
        for (bank, addr, val) in writes {
            cartridge.write(0x4000, bank);
            cartridge.write(addr, val);
        }
        let image = cartridge.save_image(now);
        prop_assert!(image.is_some());
        let mut reloaded =
            GameBoy::from_rom_with_save(rom, image.clone(), 48_000).expect("own save layout");
        prop_assert_eq!(reloaded.cartridge().save_image(now), image);
    }
}
