use super::*;

const TICKS_PER_LINE: u32 = 114;
const TICKS_PER_FRAME: u32 = TICKS_PER_LINE * 154;
const LCDC: u16 = 0xFF40;
const STAT: u16 = 0xFF41;
const LY: u16 = 0xFF44;
const LYC: u16 = 0xFF45;

/// Counts interrupt requests matching `mask` over `ticks`.
fn requests(ppu: &mut Ppu, ticks: u32, mask: u8) -> u32 {
    (0..ticks).map(|_| u32::from(ppu.tick() & mask != 0)).sum()
}

#[test]
fn tick_frame_has_one_vblank_and_wraps() {
    let mut ppu = Ppu::new();
    assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_VBLANK), 1);
    assert_eq!((ppu.read(LY), ppu.dots), (0, 0));
}

#[test]
fn tick_lyc_interrupts_once_per_frame_on_every_line() {
    for lyc in [0, 1, 143, 144, 153] {
        let mut ppu = Ppu::new();
        ppu.write(LYC, lyc);
        requests(&mut ppu, TICKS_PER_FRAME, 0);
        ppu.write(STAT, STAT_LYC_SOURCE);
        requests(&mut ppu, TICKS_PER_LINE, 0);
        assert_eq!(
            requests(&mut ppu, TICKS_PER_FRAME, IRQ_STAT),
            1,
            "LYC {lyc}"
        );
    }
}

#[test]
fn tick_lyc_out_of_range_never_interrupts() {
    let mut ppu = Ppu::new();
    ppu.write(LYC, 154);
    ppu.write(STAT, STAT_LYC_SOURCE);
    assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_STAT), 0);
}

#[test]
fn write_lyc_updates_coincidence_at_once() {
    let mut ppu = Ppu::new();
    ppu.write(LYC, 1);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, 0);
    ppu.write(STAT, STAT_LYC_SOURCE);
    ppu.write(LYC, 0);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, STAT_COINCIDENCE);
    assert_eq!(ppu.tick() & IRQ_STAT, IRQ_STAT);
}

#[test]
fn tick_mode_sources_interrupt_per_line() {
    for (source, expected) in [
        (STAT_HBLANK_SOURCE, 144),
        (STAT_OAM_SOURCE, 144),
        (STAT_VBLANK_SOURCE, 1),
    ] {
        let mut ppu = Ppu::new();
        requests(&mut ppu, TICKS_PER_LINE + 30, 0);
        ppu.write(STAT, source);
        assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_STAT), expected);
    }
}

#[test]
fn tick_stat_sources_share_one_edge() {
    let mut ppu = Ppu::new();
    requests(&mut ppu, 30, 0);
    ppu.write(LYC, 1);
    // Line 1 mode 2 follows line 0 H-Blank with no gap: the line never falls.
    ppu.write(STAT, STAT_HBLANK_SOURCE | STAT_OAM_SOURCE | STAT_LYC_SOURCE);
    assert_eq!(requests(&mut ppu, TICKS_PER_LINE, IRQ_STAT), 1);
}

#[test]
fn tick_modes_follow_dots() {
    let mut ppu = Ppu::new();
    for (ticks, mode) in [(20, 2), (1, 3), (42, 3), (1, 0), (49, 0), (1, 2)] {
        requests(&mut ppu, ticks, 0);
        assert_eq!(ppu.read(STAT) & 3, mode, "dot {}", ppu.dots);
    }
}

#[test]
fn write_lcdc_off_resets_line_and_freezes() {
    let mut ppu = Ppu::new();
    requests(&mut ppu, TICKS_PER_LINE * 5, 0);
    ppu.write(LCDC, 0);
    requests(&mut ppu, TICKS_PER_FRAME, 0);
    assert_eq!((ppu.read(LY), ppu.read(STAT) & 3), (0, 0));
}

#[test]
fn write_lcdc_on_compares_lyc() {
    let mut ppu = Ppu::new();
    ppu.write(LYC, 1);
    ppu.write(LCDC, 0);
    ppu.write(LYC, 0);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, 0, "frozen while off");
    ppu.write(LCDC, 0x80);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, STAT_COINCIDENCE);
}

#[test]
fn write_registers_read_back() {
    let mut ppu = Ppu::new();
    for addr in [
        0xFF42, 0xFF43, 0xFF45, 0xFF47, 0xFF48, 0xFF49, 0xFF4A, 0xFF4B,
    ] {
        ppu.write(addr, 0x5A);
        assert_eq!(ppu.read(addr), 0x5A, "{addr:#06x}");
    }
    ppu.write(LY, 9);
    assert_eq!(ppu.read(LY), 0);
    assert_eq!(ppu.read(0xFF50), OPEN_BUS);
}

#[test]
fn write_oam_out_of_range_is_ignored() {
    let mut ppu = Ppu::new();
    ppu.write_oam(OAM_LEN - 1, 1);
    ppu.write_oam(OAM_LEN, 2);
    assert_eq!(ppu.read(0xFE9F), 1);
}

#[test]
fn write_lcdc_on_first_line_has_no_oam_scan() {
    let mut ppu = Ppu::new();
    ppu.write(LCDC, 0);
    ppu.write(LCDC, 0x80);
    ppu.write(STAT, STAT_OAM_SOURCE);
    for (ticks, mode) in [(1, 0), (19, 0), (1, 3), (43, 0), (50, 2)] {
        requests(&mut ppu, ticks, 0);
        assert_eq!(ppu.read(STAT) & 3, mode, "dot {}", ppu.dots);
    }
}
