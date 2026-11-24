use super::*;
use crate::hardware::OPEN_BUS;

const TICKS_PER_LINE: u32 = 114;
const TICKS_PER_FRAME: u32 = TICKS_PER_LINE * 154;
const LCDC: u16 = 0xFF40;
const STAT: u16 = 0xFF41;
const LY: u16 = 0xFF44;
const LYC: u16 = 0xFF45;

/// PPU at the first dot of a frame.
fn line_0() -> Ppu {
    let mut ppu = Ppu::new();
    while ppu.line != 0 {
        ppu.tick();
    }
    ppu
}

/// Counts interrupt requests matching `mask` over `ticks`.
fn requests(ppu: &mut Ppu, ticks: u32, mask: u8) -> u32 {
    (0..ticks).map(|_| u32::from(ppu.tick() & mask != 0)).sum()
}

#[test]
fn tick_frame_has_one_vblank_and_wraps() {
    let mut ppu = line_0();
    assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_VBLANK), 1);
    assert_eq!((ppu.read(LY), ppu.dots), (0, 0));
}

#[test]
fn tick_lyc_interrupts_once_per_frame_on_every_line() {
    for lyc in [0, 1, 143, 144, 153] {
        let mut ppu = line_0();
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
    let mut ppu = line_0();
    ppu.write(LYC, 154);
    ppu.write(STAT, STAT_LYC_SOURCE);
    // The STAT write glitch itself.
    ppu.tick();
    assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_STAT), 0);
}

#[test]
fn write_lyc_updates_coincidence_at_once() {
    let mut ppu = line_0();
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
        (STAT_OAM_SOURCE, 145),
        (STAT_VBLANK_SOURCE, 1),
    ] {
        let mut ppu = line_0();
        requests(&mut ppu, TICKS_PER_LINE + 30, 0);
        ppu.write(STAT, source);
        assert_eq!(requests(&mut ppu, TICKS_PER_FRAME, IRQ_STAT), expected);
    }
}

#[test]
fn tick_stat_sources_share_one_edge() {
    let mut ppu = line_0();
    requests(&mut ppu, 30, 0);
    ppu.write(LYC, 1);
    // Line 1 mode 2 follows line 0 H-Blank with no gap: the line never falls.
    ppu.write(STAT, STAT_HBLANK_SOURCE | STAT_OAM_SOURCE | STAT_LYC_SOURCE);
    assert_eq!(requests(&mut ppu, TICKS_PER_LINE, IRQ_STAT), 1);
}

#[test]
fn tick_modes_follow_dots() {
    let mut ppu = line_0();
    for (ticks, mode) in [(20, 2), (1, 3), (42, 3), (1, 0), (49, 0), (1, 0), (1, 2)] {
        requests(&mut ppu, ticks, 0);
        assert_eq!(ppu.read(STAT) & 3, mode, "dot {}", ppu.dots);
    }
}

#[test]
fn write_lcdc_off_resets_line_and_freezes() {
    let mut ppu = line_0();
    requests(&mut ppu, TICKS_PER_LINE * 5, 0);
    ppu.write(LCDC, 0);
    requests(&mut ppu, TICKS_PER_FRAME, 0);
    assert_eq!((ppu.read(LY), ppu.read(STAT) & 3), (0, 0));
}

#[test]
fn write_lcdc_on_compares_lyc() {
    let mut ppu = line_0();
    ppu.write(LYC, 1);
    ppu.write(LCDC, 0);
    ppu.write(LYC, 0);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, 0, "frozen while off");
    ppu.write(LCDC, 0x80);
    assert_eq!(ppu.read(STAT) & STAT_COINCIDENCE, STAT_COINCIDENCE);
}

#[test]
fn write_registers_read_back() {
    let mut ppu = line_0();
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
    let mut ppu = line_0();
    ppu.write(LCDC, 0);
    ppu.write_oam(OAM_LEN - 1, 1);
    ppu.write_oam(OAM_LEN, 2);
    assert_eq!(ppu.read(0xFE9F), 1);
}

#[test]
fn write_lcdc_on_first_line_has_no_oam_scan() {
    let mut ppu = line_0();
    ppu.write(LCDC, 0);
    ppu.write(LCDC, 0x80);
    ppu.write(STAT, STAT_OAM_SOURCE);
    for (ticks, mode) in [(1, 0), (18, 0), (1, 3), (43, 0), (51, 2)] {
        requests(&mut ppu, ticks, 0);
        assert_eq!(ppu.read(STAT) & 3, mode, "dot {}", ppu.dots);
    }
}

#[test]
fn tick_last_line_reports_zero_early() {
    let mut ppu = line_0();
    requests(&mut ppu, TICKS_PER_LINE * 153, 0);
    assert_eq!(ppu.read(LY), 153);
    requests(&mut ppu, 1, 0);
    assert_eq!(ppu.read(LY), 0);
    requests(&mut ppu, TICKS_PER_LINE, 0);
    assert_eq!((ppu.read(LY), ppu.line), (0, 0));
}

#[test]
fn write_stat_glitch_interrupts_in_blanking_only() {
    for (ticks, expected) in [(10, 0), (30, 0), (80, 1), (TICKS_PER_LINE * 150, 1)] {
        let mut ppu = line_0();
        ppu.write(LYC, 200);
        requests(&mut ppu, ticks, 0);
        ppu.write(STAT, 0);
        assert_eq!(requests(&mut ppu, 1, IRQ_STAT), expected, "tick {ticks}");
    }
}

#[test]
fn write_lcdc_on_line_is_not_an_hblank_source() {
    let mut ppu = line_0();
    ppu.write(LCDC, 0);
    ppu.write(LYC, 9);
    ppu.write(STAT, STAT_HBLANK_SOURCE);
    ppu.tick();
    ppu.write(LCDC, 0x80);
    assert_eq!(requests(&mut ppu, 60, IRQ_STAT), 0);
    assert_eq!(requests(&mut ppu, 10, IRQ_STAT), 1);
}

#[test]
fn read_video_memory_follows_mode() {
    let mut ppu = line_0();
    requests(&mut ppu, TICKS_PER_LINE, 0);
    for (ticks, vram, oam) in [(1, 0, OPEN_BUS), (24, OPEN_BUS, OPEN_BUS), (60, 0, 0)] {
        requests(&mut ppu, ticks, 0);
        assert_eq!((ppu.read(0x8000), ppu.read(0xFE00)), (vram, oam));
    }
}

#[test]
fn write_video_memory_while_drawing_is_ignored() {
    let mut ppu = line_0();
    requests(&mut ppu, 25, 0);
    ppu.write(0x8000, 1);
    ppu.write(0xFE00, 1);
    ppu.write_oam(1, 2);
    requests(&mut ppu, 60, 0);
    assert_eq!(
        (ppu.read(0x8000), ppu.read(0xFE00), ppu.read(0xFE01)),
        (0, 0, 2)
    );
}

#[test]
fn frame_shows_complete_pictures_only() {
    let mut ppu = line_0();
    ppu.write(0xFF47, 0xFF);
    requests(&mut ppu, TICKS_PER_LINE * 144, 0);
    assert!(ppu.frame().iter().all(|&byte| byte == 0));
    requests(&mut ppu, 1, 0);
    assert_eq!(ppu.frame()[..4], [8, 24, 32, 255]);
}

#[test]
fn frame_is_blank_while_off_and_one_picture_after() {
    let mut ppu = line_0();
    ppu.write(0xFF47, 0xFF);
    requests(&mut ppu, TICKS_PER_FRAME, 0);
    ppu.write(LCDC, 0);
    assert_eq!(ppu.frame()[..4], [224, 248, 208, 255]);
    ppu.write(LCDC, 0x91);
    requests(&mut ppu, TICKS_PER_FRAME, 0);
    assert_eq!(ppu.frame()[..4], [224, 248, 208, 255]);
    requests(&mut ppu, TICKS_PER_FRAME, 0);
    assert_eq!(ppu.frame()[..4], [8, 24, 32, 255]);
}

#[test]
fn read_write_video_memory_at_mode_boundaries() {
    let mut ppu = line_0();
    requests(&mut ppu, TICKS_PER_LINE, 0);
    assert_eq!((ppu.read(STAT) & 3, ppu.read(0xFE00)), (0, OPEN_BUS));
    requests(&mut ppu, 20, 0);
    assert_eq!((ppu.read(STAT) & 3, ppu.read(0x8000)), (2, OPEN_BUS));
    ppu.write(0x8000, 3);
    ppu.write(0xFE00, 4);
    requests(&mut ppu, 60, 0);
    assert_eq!((ppu.read(0x8000), ppu.read(0xFE00)), (3, 4));
}

#[test]
fn write_stat_oam_source_during_scan_waits_for_next_line() {
    let mut ppu = line_0();
    requests(&mut ppu, TICKS_PER_LINE + 6, 0);
    ppu.write(STAT, STAT_OAM_SOURCE);
    assert_eq!(requests(&mut ppu, TICKS_PER_LINE - 7, IRQ_STAT), 0);
    assert_eq!(requests(&mut ppu, 1, IRQ_STAT), 1);
}

#[test]
fn tick_oam_source_leads_mode_except_on_line_0() {
    for (lines, ticks_to_request) in [(1, 0), (2, 0), (144, 0), (154, 1)] {
        let mut ppu = line_0();
        ppu.write(STAT, STAT_OAM_SOURCE);
        requests(&mut ppu, TICKS_PER_LINE * lines - 1, 0);
        assert_eq!(requests(&mut ppu, ticks_to_request, IRQ_STAT), 0);
        assert_eq!(requests(&mut ppu, 1, IRQ_STAT), 1, "line {lines}");
    }
}
