//! Scanline composition: background, window and sprites.

use super::{Ppu, DOTS_PER_TICK, PIXEL_LEN, ROW_LEN, SCREEN_WIDTH, VRAM_BASE};

const LCDC_WINDOW_MAP: u8 = 0x40;
const LCDC_WINDOW_ON: u8 = 0x20;
const LCDC_UNSIGNED_TILES: u8 = 0x10;
const LCDC_BG_MAP: u8 = 0x08;
const LCDC_TALL_SPRITES: u8 = 0x04;
const LCDC_SPRITES_ON: u8 = 0x02;
const LCDC_BG_ON: u8 = 0x01;

const MAP_LOW: u16 = 0x9800;
const MAP_HIGH: u16 = 0x9C00;
const MAP_TILES_PER_ROW: u16 = 32;
const TILE_SIZE_PX: u8 = 8;
const TILE_LEN: u16 = 16;
/// Tile 0 of the signed addressing mode.
const SIGNED_TILE_BASE: u16 = 0x9000;

/// The window starts 7 pixels left of WX.
const WINDOW_X_OFFSET: u8 = 7;
/// Largest WX that still shows the window.
const WINDOW_MAX_X: u8 = 166;

const SPRITE_LEN: usize = 4;
const MAX_SPRITES_PER_LINE: usize = 10;
const SPRITE_Y_OFFSET: u8 = 16;
const SPRITE_FETCH_DOTS: u16 = 6;
const SPRITE_MAX_WAIT_DOTS: u16 = 5;
const SPRITE_X_OFFSET: u8 = 8;
const SPRITE_BEHIND_BG: u8 = 0x80;
const SPRITE_FLIP_Y: u8 = 0x40;
const SPRITE_FLIP_X: u8 = 0x20;
const SPRITE_PALETTE_1: u8 = 0x10;

/// Shades of the DMG screen, lightest first.
const SHADES: [[u8; PIXEL_LEN]; 4] = [
    [224, 248, 208, 255],
    [136, 192, 112, 255],
    [52, 104, 86, 255],
    [8, 24, 32, 255],
];

/// One value per pixel of a line.
type Line = [u8; SCREEN_WIDTH];

/// Both bit planes of eight horizontal pixels.
#[derive(Clone, Copy, Debug, Default)]
struct TileRow {
    low: u8,
    high: u8,
}

impl TileRow {
    /// Color index of pixel `x` (0-7) from the left.
    fn color(self, x: u8) -> u8 {
        let shift = u32::from(TILE_SIZE_PX - 1 - (x % TILE_SIZE_PX));
        ((self.low >> shift) & 1) | (((self.high >> shift) & 1) << 1)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Sprite {
    y: u8,
    x: u8,
    tile: u8,
    flags: u8,
}

/// Sprites on one line, highest priority first.
struct LineSprites {
    sprites: [Sprite; MAX_SPRITES_PER_LINE],
    len: usize,
}

impl Ppu {
    /// Shows the lightest shade everywhere, as with the LCD off.
    pub(super) fn blank_frame(&mut self) {
        let [lightest, ..] = SHADES;
        self.frame.as_chunks_mut::<PIXEL_LEN>().0.fill(lightest);
    }

    /// Dots the current line extends mode 3, on the M-cycle grid.
    pub(super) fn drawing_penalty(&self) -> u16 {
        let scroll = u16::from(self.scx % TILE_SIZE_PX).next_multiple_of(DOTS_PER_TICK);
        let sprites = if self.lcdc & LCDC_SPRITES_ON != 0 {
            self.sprite_penalty()
        } else {
            0
        };
        scroll + (sprites - sprites % DOTS_PER_TICK)
    }

    /// Each sprite costs its fetch plus, once per background tile, the wait for that
    /// tile's remaining pixels.
    fn sprite_penalty(&self) -> u16 {
        let found = self.line_sprites();
        let mut last_tile = None;
        let visible = found.sprites.iter().take(found.len);
        visible
            .filter(|sprite| usize::from(sprite.x) < SCREEN_WIDTH + usize::from(SPRITE_X_OFFSET))
            .map(|sprite| {
                let position = u16::from(sprite.x) + u16::from(self.scx % TILE_SIZE_PX);
                let tile = position / u16::from(TILE_SIZE_PX);
                let offset = position % u16::from(TILE_SIZE_PX);
                let wait = if last_tile == Some(tile) {
                    0
                } else {
                    SPRITE_MAX_WAIT_DOTS.saturating_sub(offset)
                };
                last_tile = Some(tile);
                SPRITE_FETCH_DOTS + wait
            })
            .sum()
    }

    /// Draws the current line into the back buffer.
    pub(super) fn render_line(&mut self) {
        self.window_triggered |= self.ly == self.wy;
        let mut colors = [0; SCREEN_WIDTH];
        if self.lcdc & LCDC_BG_ON != 0 {
            self.draw_background(&mut colors);
        }
        let mut shades = colors.map(|color| shade(self.bgp, color));
        if self.lcdc & LCDC_SPRITES_ON != 0 {
            self.draw_sprites(&colors, &mut shades);
        }
        let (rows, _) = self.back.as_chunks_mut::<ROW_LEN>();
        let Some(row) = rows.get_mut(usize::from(self.ly)) else {
            return;
        };
        for (pixel, shade) in row.as_chunks_mut::<PIXEL_LEN>().0.iter_mut().zip(shades) {
            if let Some(rgba) = SHADES.get(usize::from(shade)) {
                *pixel = *rgba;
            }
        }
    }

    fn draw_background(&mut self, colors: &mut Line) {
        let map = self.map_base(LCDC_BG_MAP);
        self.draw_tiles(colors, map, self.scx, self.ly.wrapping_add(self.scy));

        if !self.window_visible() {
            return;
        }
        let start = usize::from(self.wx.saturating_sub(WINDOW_X_OFFSET));
        let hidden = WINDOW_X_OFFSET.saturating_sub(self.wx);
        if let Some(visible) = colors.get_mut(start..) {
            let map = self.map_base(LCDC_WINDOW_MAP);
            self.draw_tiles(visible, map, hidden, self.window_line);
        }
        self.window_line += 1;
    }

    fn window_visible(&self) -> bool {
        self.lcdc & LCDC_WINDOW_ON != 0 && self.window_triggered && self.wx <= WINDOW_MAX_X
    }

    /// Fills `colors` from a tile map, starting at map coordinates `x`, `y`.
    fn draw_tiles(&self, colors: &mut [u8], map: u16, x: u8, y: u8) {
        let row_base = map + u16::from(y / TILE_SIZE_PX) * MAP_TILES_PER_ROW;
        let mut x = x;
        let mut row = TileRow::default();
        let mut row_column = None;
        for color in colors {
            let column = x / TILE_SIZE_PX;
            if row_column != Some(column) {
                let tile = self.vram_byte(row_base + u16::from(column));
                row = self.tile_row(self.tile_base(tile), y % TILE_SIZE_PX);
                row_column = Some(column);
            }
            *color = row.color(x);
            x = x.wrapping_add(1);
        }
    }

    fn draw_sprites(&self, colors: &Line, shades: &mut Line) {
        let mut claimed = [false; SCREEN_WIDTH];
        let line_sprites = self.line_sprites();
        for sprite in line_sprites.sprites.iter().take(line_sprites.len) {
            let row = self.sprite_row(*sprite);
            let palette = if sprite.flags & SPRITE_PALETTE_1 != 0 {
                self.obp1
            } else {
                self.obp0
            };
            // Off-screen columns: left of the screen, then right of it.
            let hidden = SPRITE_X_OFFSET.saturating_sub(sprite.x);
            let start = usize::from(sprite.x.saturating_sub(SPRITE_X_OFFSET));
            let pixels = colors.iter().zip(shades.iter_mut()).zip(&mut claimed);
            for (((&bg, out), claimed), x) in pixels.skip(start).zip(hidden..TILE_SIZE_PX) {
                let flipped = sprite.flags & SPRITE_FLIP_X != 0;
                let color = row.color(if flipped { TILE_SIZE_PX - 1 - x } else { x });
                if color == 0 || *claimed {
                    continue;
                }
                // Hardware picks the winning sprite before testing it against the
                // background, so a hidden winner still masks the sprites below it.
                *claimed = true;
                if sprite.flags & SPRITE_BEHIND_BG == 0 || bg == 0 {
                    *out = shade(palette, color);
                }
            }
        }
    }

    /// The first ten sprites in OAM order, sorted by X; OAM order breaks ties.
    fn line_sprites(&self) -> LineSprites {
        let mut found = LineSprites {
            sprites: [Sprite::default(); MAX_SPRITES_PER_LINE],
            len: 0,
        };
        let on_line = self
            .oam
            .as_chunks::<SPRITE_LEN>()
            .0
            .iter()
            .map(|&[y, x, tile, flags]| Sprite { y, x, tile, flags })
            .filter(|sprite| self.sprite_line(*sprite).is_some());
        for (slot, sprite) in found.sprites.iter_mut().zip(on_line) {
            *slot = sprite;
            found.len += 1;
        }
        if let Some(used) = found.sprites.get_mut(..found.len) {
            // Stable, and allocation-free at this length (insertion sort).
            used.sort_by_key(|sprite| sprite.x);
        }
        found
    }

    /// Line of the sprite on the current scanline, before flipping.
    fn sprite_line(&self, sprite: Sprite) -> Option<u8> {
        (self.ly + SPRITE_Y_OFFSET)
            .checked_sub(sprite.y)
            .filter(|&line| line < self.sprite_height())
    }

    fn sprite_height(&self) -> u8 {
        if self.lcdc & LCDC_TALL_SPRITES != 0 {
            2 * TILE_SIZE_PX
        } else {
            TILE_SIZE_PX
        }
    }

    fn sprite_row(&self, sprite: Sprite) -> TileRow {
        let height = self.sprite_height();
        let line = self.sprite_line(sprite).unwrap_or(0);
        let line = if sprite.flags & SPRITE_FLIP_Y != 0 {
            height - 1 - line
        } else {
            line
        };
        let tile = if height == TILE_SIZE_PX {
            sprite.tile
        } else {
            // The two tiles of a tall sprite are consecutive, the first one even.
            sprite.tile & !1
        };
        self.tile_row(VRAM_BASE + u16::from(tile) * TILE_LEN, line)
    }

    fn tile_row(&self, tile_base: u16, line: u8) -> TileRow {
        let addr = tile_base + u16::from(line) * 2;
        TileRow {
            low: self.vram_byte(addr),
            high: self.vram_byte(addr + 1),
        }
    }

    fn tile_base(&self, tile: u8) -> u16 {
        if self.lcdc & LCDC_UNSIGNED_TILES != 0 {
            VRAM_BASE + u16::from(tile) * TILE_LEN
        } else {
            let offset = i16::from(i8::from_ne_bytes([tile])) << TILE_LEN.trailing_zeros();
            SIGNED_TILE_BASE.wrapping_add_signed(offset)
        }
    }

    fn map_base(&self, select: u8) -> u16 {
        if self.lcdc & select != 0 {
            MAP_HIGH
        } else {
            MAP_LOW
        }
    }
}

/// Applies a palette register to a color index.
fn shade(palette: u8, color: u8) -> u8 {
    (palette >> ((color & 3) * 2)) & 3
}

#[cfg(test)]
mod tests {
    use super::super::Mode;
    use super::*;

    const SOLID: u8 = 1;

    /// In H-Blank, so video memory is accessible; tile 1 is solid color 3; BG shows tile 0 (blank); identity palettes.
    fn ppu() -> Ppu {
        let mut ppu = Ppu::new();
        ppu.mode = Mode::HBlank;
        for offset in 0..16 {
            ppu.write(0x8010 + offset, 0xFF);
        }
        ppu.lcdc = 0x93;
        ppu.write(0xFF47, 0xE4);
        ppu.write(0xFF48, 0xE4);
        ppu.write(0xFF49, 0x1B);
        ppu
    }

    fn put_sprite(ppu: &mut Ppu, idx: usize, sprite: [u8; 4]) {
        for (offset, val) in sprite.into_iter().enumerate() {
            ppu.write_oam(idx * 4 + offset, val);
        }
    }

    /// Shade indices of the first rendered line.
    fn line(ppu: &mut Ppu) -> Vec<usize> {
        ppu.render_line();
        ppu.back[..ROW_LEN]
            .as_chunks::<PIXEL_LEN>()
            .0
            .iter()
            .map(|pixel| SHADES.iter().position(|s| s == pixel).expect("known shade"))
            .collect()
    }

    fn lit(ppu: &mut Ppu) -> Vec<usize> {
        let line = line(ppu);
        (0..SCREEN_WIDTH).filter(|&x| line[x] != 0).collect()
    }

    #[test]
    fn shade_selects_palette_entry() {
        assert_eq!([0, 1, 2, 3].map(|color| shade(0x1B, color)), [3, 2, 1, 0]);
    }

    #[test]
    fn color_combines_planes_left_to_right() {
        let row = TileRow {
            low: 0b1010_0000,
            high: 0b1100_0000,
        };
        assert_eq!([0, 1, 2, 3].map(|x| row.color(x)), [3, 2, 1, 0]);
    }

    #[test]
    fn draw_background_scrolls_and_wraps() {
        let mut ppu = ppu();
        ppu.write(0x9800, SOLID);
        ppu.write(0xFF43, 0xFC);
        assert_eq!(lit(&mut ppu), (4..12).collect::<Vec<_>>());
    }

    #[test]
    fn draw_background_disabled_is_color_zero() {
        let mut ppu = ppu();
        ppu.write(0x9800, SOLID);
        ppu.lcdc = 0x92;
        assert_eq!(lit(&mut ppu), []);
    }

    #[test]
    fn tile_base_addresses_both_modes() {
        let mut ppu = ppu();
        for (lcdc, tile, expected) in [
            (0x10, 0xFF, 0x8FF0),
            (0x00, 0x00, 0x9000),
            (0x00, 0x7F, 0x97F0),
            (0x00, 0x80, 0x8800),
        ] {
            ppu.lcdc = lcdc;
            assert_eq!(ppu.tile_base(tile), expected);
        }
    }

    #[test]
    fn draw_background_window_covers_from_wx() {
        for (wx, expected) in [
            (7, 0..8),
            (3, 0..4),
            (27, 20..28),
            (166, 159..160),
            (167, 0..0),
        ] {
            let mut ppu = ppu();
            ppu.write(0x9C00, SOLID);
            ppu.lcdc = 0xF1;
            ppu.write(0xFF4B, wx);
            assert_eq!(lit(&mut ppu), expected.collect::<Vec<_>>(), "WX {wx}");
            assert_eq!(ppu.window_line, u8::from(wx <= 166));
        }
    }

    #[test]
    fn draw_sprites_clips_at_screen_edges() {
        for (x, expected) in [
            (0, 0..0),
            (4, 0..4),
            (8, 0..8),
            (164, 156..160),
            (168, 0..0),
        ] {
            let mut ppu = ppu();
            put_sprite(&mut ppu, 0, [16, x, SOLID, 0]);
            assert_eq!(lit(&mut ppu), expected.collect::<Vec<_>>(), "X {x}");
        }
    }

    #[test]
    fn draw_sprites_limit_is_ten_in_oam_order() {
        let mut ppu = ppu();
        for idx in 0..12u8 {
            put_sprite(&mut ppu, usize::from(idx), [16, 160 - idx * 8, SOLID, 0]);
        }
        assert_eq!(lit(&mut ppu).len(), 80);
        assert_eq!(lit(&mut ppu).first(), Some(&80));
    }

    #[test]
    fn draw_sprites_hidden_by_x_still_count_towards_limit() {
        let mut ppu = ppu();
        (0..10).for_each(|idx| put_sprite(&mut ppu, idx, [16, 0, SOLID, 0]));
        put_sprite(&mut ppu, 10, [16, 8, SOLID, 0]);
        assert_eq!(lit(&mut ppu), []);
    }

    #[test]
    fn draw_sprites_lower_x_wins_then_oam_order() {
        for (first, second, expected) in [(12, 8, 3), (8, 12, 1), (8, 8, 1)] {
            let mut ppu = ppu();
            ppu.write(0xFF49, 0xE4);
            ppu.write(0xFF48, 0x40);
            put_sprite(&mut ppu, 0, [16, first, SOLID, 0]);
            put_sprite(&mut ppu, 1, [16, second, SOLID, SPRITE_PALETTE_1]);
            assert_eq!(line(&mut ppu)[4], expected, "{first} {second}");
        }
    }

    #[test]
    fn draw_sprites_hidden_winner_masks_lower_priority() {
        let mut ppu = ppu();
        ppu.write(0x9800, SOLID);
        put_sprite(&mut ppu, 0, [16, 8, SOLID, SPRITE_BEHIND_BG]);
        put_sprite(&mut ppu, 1, [16, 8, SOLID, SPRITE_PALETTE_1]);
        assert_eq!(line(&mut ppu)[0], 3, "background stays on top");
    }

    #[test]
    fn sprite_row_flips_and_spans_two_tiles() {
        let mut ppu = ppu();
        ppu.write(0x8020, 0x80);
        ppu.write(0x803E, 0x01);
        ppu.lcdc |= LCDC_TALL_SPRITES;
        for (flags, expected) in [
            (0, 0),
            (SPRITE_FLIP_X, 7),
            (SPRITE_FLIP_Y, 7),
            (SPRITE_FLIP_X | SPRITE_FLIP_Y, 0),
        ] {
            put_sprite(&mut ppu, 0, [16, 8, 3, flags]);
            assert_eq!(lit(&mut ppu), [expected], "{flags:#x}");
        }
    }

    #[test]
    fn sprite_line_covers_height_only() {
        let mut ppu = ppu();
        for (ly, y, expected) in [
            (0, 16, Some(0)),
            (0, 9, Some(7)),
            (0, 8, None),
            (0, 17, None),
        ] {
            ppu.ly = ly;
            let sprite = Sprite {
                y,
                ..Sprite::default()
            };
            assert_eq!(ppu.sprite_line(sprite), expected);
        }
    }

    #[test]
    fn window_triggers_on_exact_line_only() {
        for (wy, start, triggered) in [(5, 5, true), (4, 5, false), (6, 5, false)] {
            let mut ppu = ppu();
            ppu.ly = start;
            ppu.write(0xFF4A, wy);
            ppu.render_line();
            assert_eq!(ppu.window_triggered, triggered);
        }
    }

    #[test]
    fn window_already_triggered_ignores_wy() {
        let mut ppu = ppu();
        ppu.write(0x9C00, SOLID);
        ppu.lcdc = 0xF1;
        ppu.write(0xFF4B, 7);
        ppu.render_line();
        ppu.write(0xFF4A, 100);
        ppu.render_line();
        assert_eq!(ppu.window_line, 2);
    }

    #[test]
    fn drawing_penalty_counts_scroll_and_sprites() {
        for (scx, sprites, expected) in [
            (0, &[][..], 0),
            (1, &[], 4),
            (5, &[], 8),
            (8, &[], 0),
            (0, &[0], 8),
            (0, &[5], 4),
            (0, &[168], 0),
            (0, &[0, 0], 16),
            (0, &[0, 8], 20),
        ] {
            let mut ppu = ppu();
            ppu.write(0xFF43, scx);
            for (idx, &x) in sprites.iter().enumerate() {
                put_sprite(&mut ppu, idx, [16, x, SOLID, 0]);
            }
            assert_eq!(ppu.drawing_penalty(), expected, "{scx} {sprites:?}");
        }
    }
}
