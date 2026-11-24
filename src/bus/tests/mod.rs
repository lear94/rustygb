use super::*;

/// Machine with the LCD off, so video memory is accessible.
fn bus() -> Motherboard {
    let cartridge = Cartridge::from_bytes(vec![0; 0x8000], None).expect("blank ROM is valid");
    let mut bus = Motherboard::new(cartridge, 48_000);
    bus.write_byte(0xFF40, 0);
    bus
}

#[test]
fn write_byte_ram_reads_back() {
    let mut bus = bus();
    for addr in [
        0x8000, 0x9FFF, 0xC000, 0xDFFF, 0xFE00, 0xFE9F, 0xFF80, 0xFFFE, 0xFFFF,
    ] {
        bus.write_byte(addr, 0x5A);
        assert_eq!(bus.read_byte(addr), 0x5A, "{addr:#06x}");
    }
}

#[test]
fn write_byte_echo_mirrors_wram() {
    let mut bus = bus();
    bus.write_byte(0xE000, 1);
    bus.write_byte(0xDDFF, 2);
    assert_eq!((bus.read_byte(0xC000), bus.read_byte(0xFDFF)), (1, 2));
}

#[test]
fn read_byte_unmapped_is_open_bus() {
    let mut bus = bus();
    for addr in [0xA000, 0xFF03, 0xFF4C, 0xFF7F] {
        bus.write_byte(addr, 0);
        assert_eq!(bus.read_byte(addr), OPEN_BUS, "{addr:#06x}");
    }
}

#[test]
fn acknowledge_interrupt_takes_lowest_enabled() {
    let mut bus = bus();
    bus.write_byte(0xFF0F, 0x15);
    bus.write_byte(0xFFFF, 0xF4);
    assert_eq!(bus.pending_interrupts(), 0x14);
    assert_eq!(bus.acknowledge_interrupt(), Some(2));
    assert_eq!(bus.acknowledge_interrupt(), Some(4));
    assert_eq!(bus.acknowledge_interrupt(), None);
}

#[test]
fn tick_dma_copies_and_blocks_oam() {
    let mut bus = bus();
    for offset in 0..160u8 {
        bus.write_byte(0xC100 + u16::from(offset), offset);
    }
    bus.write_byte(0xFF46, 0xC1);
    bus.tick();
    bus.tick();
    assert_eq!(bus.read_byte(0xFE00), OPEN_BUS);
    bus.write_byte(0xFE00, 0x77);
    (0..160).for_each(|_| bus.tick());
    assert_eq!((bus.read_byte(0xFE00), bus.read_byte(0xFE9F)), (0, 159));
    assert_eq!(bus.read_byte(0xFF46), 0xC1);
}

#[test]
fn tick_timer_overflow_requests_interrupt() {
    let mut bus = bus();
    bus.write_byte(0xFF0F, 0);
    bus.write_byte(0xFF05, 0xFF);
    bus.write_byte(0xFF07, 5);
    (0..8).for_each(|_| bus.tick());
    assert_eq!(bus.read_byte(0xFF0F) & IRQ_TIMER, IRQ_TIMER);
}

#[test]
fn read_byte_unusable_area_is_zero() {
    for addr in [0xFEA0, 0xFEFF] {
        assert_eq!(bus().read_byte(addr), 0);
    }
}

#[test]
fn tick_serial_transfer_follows_divider() {
    let mut bus = bus();
    bus.write_byte(0xFF04, 0);
    bus.tick();
    bus.write_byte(0xFF0F, 0);
    bus.write_byte(0xFF02, 0x81);
    (0..1022).for_each(|_| bus.tick());
    assert_eq!(bus.read_byte(0xFF0F) & IRQ_SERIAL, 0);
    bus.tick();
    assert_eq!(bus.read_byte(0xFF0F) & IRQ_SERIAL, IRQ_SERIAL);
}

#[test]
fn tick_stopped_produces_silence() {
    let mut bus = bus();
    (0..1_048_576 / 8).for_each(|_| bus.tick_stopped());
    let mut samples = vec![1.0; 6_001];
    assert_eq!(bus.apu.drain_samples(&mut samples), 6_000);
    assert!(samples[..6_000]
        .iter()
        .all(|sample| sample.abs() < f32::EPSILON));
}
