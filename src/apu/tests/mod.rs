use super::*;

const SAMPLE_RATE_HZ: u32 = 48_000;
const SEQUENCER_PERIOD_TICKS: u32 = 2048;

/// Ticks with the sequencer clocked at 512 Hz, starting a full period before its edge.
fn tick(apu: &mut Apu, ticks: u32) {
    for idx in 1..=ticks {
        apu.tick(idx % SEQUENCER_PERIOD_TICKS == 0);
    }
}

fn apu() -> Apu {
    Apu::new(SAMPLE_RATE_HZ)
}

fn play_square_2(apu: &mut Apu, frequency: u16) {
    let [high, low] = frequency.to_be_bytes();
    apu.write(0xFF16, 0x80, false);
    apu.write(0xFF17, 0xF0, false);
    apu.write(0xFF18, low, false);
    apu.write(0xFF19, 0x80 | high, false);
}

fn run(apu: &mut Apu, ticks: u32) -> Vec<f32> {
    let mut out = Vec::new();
    for idx in 1..=ticks {
        apu.tick(idx % SEQUENCER_PERIOD_TICKS == 0);
        let mut slot = [0.0];
        if apu.drain_samples(&mut slot) == 1 {
            out.push(slot[0]);
        }
    }
    out
}

#[test]
fn tick_produces_exact_sample_count() {
    for rate in [0, 1, 44_100, SAMPLE_RATE_HZ, u32::MAX] {
        let mut apu = Apu::new(rate);
        let expected = usize::try_from(rate.min(M_CYCLES_PER_SECOND)).expect("fits");
        assert_eq!(run(&mut apu, M_CYCLES_PER_SECOND).len(), expected);
    }
}

#[test]
fn tick_square_pitch_is_exact() {
    let mut apu = apu();
    play_square_2(&mut apu, 0x7F9);
    let samples = run(&mut apu, M_CYCLES_PER_SECOND);
    let rises = samples.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0);
    // 131072 / 7 Hz, below the 24 kHz Nyquist limit.
    assert!((18_723..=18_725).contains(&rises.count()));
}

#[test]
fn mix_stays_within_unit_range() {
    let mut apu = apu();
    apu.write(NR50, 0x77, false);
    apu.write(NR51, 0xFF, false);
    for base in [0xFF11, 0xFF16] {
        apu.write(base + 1, 0xF0, false);
        apu.write(base + 3, 0x87, false);
    }
    apu.write(0xFF1A, 0x80, false);
    apu.write(0xFF1C, 0x20, false);
    apu.write(0xFF1E, 0x87, false);
    apu.write(0xFF21, 0xF0, false);
    apu.write(0xFF23, 0x80, false);
    assert_eq!(apu.read(NR52), 0xFF);
    let samples = run(&mut apu, M_CYCLES_PER_SECOND / 4);
    assert!(samples.iter().all(|s| s.abs() <= 1.0));
    assert!(samples.iter().any(|s| s.abs() > 0.5));
}

#[test]
fn mix_unpanned_channel_is_silent() {
    let mut apu = apu();
    play_square_2(&mut apu, 0x700);
    apu.write(NR51, 0x11, false);
    assert!(run(&mut apu, 10_000).iter().all(|s| s.abs() < f32::EPSILON));
}

#[test]
fn drain_samples_queue_is_bounded() {
    let mut apu = apu();
    tick(&mut apu, M_CYCLES_PER_SECOND);
    assert_eq!(apu.buffered_samples(), MAX_BUFFERED_SAMPLES);
    assert_eq!(
        apu.samples.capacity(),
        VecDeque::<f32>::with_capacity(MAX_BUFFERED_SAMPLES).capacity()
    );
    let mut out = vec![0.0; MAX_BUFFERED_SAMPLES + 1];
    assert_eq!(apu.drain_samples(&mut out), MAX_BUFFERED_SAMPLES);
    assert_eq!(apu.drain_samples(&mut out), 0);
}

#[test]
fn read_masks_unreadable_bits() {
    let mut apu = apu();
    for (addr, &mask) in (REGISTER_BASE..NR52).zip(&READ_MASKS) {
        apu.write(addr, 0, false);
        assert_eq!(apu.read(addr), mask, "{addr:#06x}");
    }
    assert_eq!(apu.read(0xFF27), 0xFF);
}

#[test]
fn read_status_tracks_channels() {
    let mut apu = apu();
    assert_eq!(apu.read(NR52), 0xF1);
    play_square_2(&mut apu, 0);
    assert_eq!(apu.read(NR52), 0xF3);
    apu.write(0xFF17, 0, false);
    assert_eq!(apu.read(NR52), 0xF1);
}

#[test]
fn write_power_off_clears_registers_but_not_wave_ram() {
    let mut apu = apu();
    apu.write(0xFF30, 0x12, false);
    apu.write(NR52, 0, false);
    assert_eq!((apu.read(NR52), apu.read(NR50)), (0x70, 0));
    apu.write(NR50, 0x77, false);
    assert_eq!(apu.read(NR50), 0, "ignored while off");
    assert_eq!(apu.read(0xFF30), 0x12);
    apu.write(NR52, 0x80, false);
    assert_eq!(apu.read(NR52), 0xF0);
}

#[test]
fn tick_powered_off_still_emits_silence() {
    let mut apu = apu();
    apu.write(NR52, 0, false);
    let samples = run(&mut apu, M_CYCLES_PER_SECOND / 8);
    assert_eq!(samples.len(), 6000);
    assert!(samples.iter().all(|s| s.abs() < f32::EPSILON));
}

#[test]
fn tick_sequencer_expires_length_at_256_hz() {
    let mut apu = apu();
    apu.write(NR52, 0, false);
    apu.write(NR52, 0x80, false);
    apu.write(0xFF16, 0x3E, false);
    apu.write(0xFF17, 0xF0, false);
    apu.write(0xFF19, 0xC0, false);
    let two_clocks = SEQUENCER_PERIOD_TICKS * 3;
    tick(&mut apu, two_clocks - 1);
    assert_eq!(apu.read(NR52), 0xF2);
    apu.tick(true);
    assert_eq!(apu.read(NR52), 0xF0);
}

#[test]
fn tick_sequencer_clocks_envelope_at_64_hz() {
    let mut apu = apu();
    apu.write(NR52, 0, false);
    apu.write(NR52, 0x80, false);
    apu.write(0xFF21, 0xF1, false);
    apu.write(0xFF23, 0x80, false);
    let cycle = SEQUENCER_PERIOD_TICKS * 8;
    tick(&mut apu, cycle - 1);
    assert_eq!(apu.noise.envelope.volume(), 15);
    apu.tick(true);
    assert_eq!(apu.noise.envelope.volume(), 14);
}

#[test]
fn write_length_while_off_is_kept() {
    let mut apu = apu();
    apu.write(NR52, 0, false);
    apu.write(0xFF16, 0x3F, false);
    apu.write(NR52, 0x80, false);
    apu.write(0xFF17, 0xF0, false);
    apu.write(0xFF19, 0xC0, false);
    tick(&mut apu, SEQUENCER_PERIOD_TICKS);
    assert_eq!(apu.read(NR52), 0xF0);
}

#[test]
fn write_power_off_stops_every_channel() {
    let mut apu = apu();
    play_square_2(&mut apu, 0);
    apu.write(0xFF1A, 0x80, false);
    apu.write(0xFF1E, 0x80, false);
    apu.write(0xFF21, 0xF0, false);
    apu.write(0xFF23, 0x80, false);
    assert_eq!(apu.read(NR52), 0xFF);
    apu.write(NR52, 0, false);
    apu.write(NR52, 0x80, false);
    assert_eq!(apu.read(NR52), 0xF0);
}

#[test]
fn write_power_on_clock_phase_sets_length_timing() {
    for (clock_high, edges) in [(false, 3), (true, 2)] {
        let mut apu = apu();
        apu.write(NR52, 0, false);
        apu.write(0xFF16, 0x3E, false);
        apu.write(NR52, 0x80, clock_high);
        apu.write(0xFF17, 0xF0, false);
        apu.write(0xFF19, 0xC0, false);
        tick(&mut apu, SEQUENCER_PERIOD_TICKS * (edges - 1));
        assert_eq!(apu.read(NR52), 0xF2, "{clock_high}");
        tick(&mut apu, SEQUENCER_PERIOD_TICKS);
        assert_eq!(apu.read(NR52), 0xF0, "{clock_high}");
    }
}

#[test]
fn emit_ultrasonic_channel_does_not_alias() {
    let mut apu = apu();
    play_square_2(&mut apu, 0x7FF);
    let samples = run(&mut apu, M_CYCLES_PER_SECOND / 10);
    // 131 kHz is inaudible; point sampling turned it into a tone at 0.25.
    assert!(samples[1..].iter().all(|sample| sample.abs() < 0.07));
}
