use winit::event::VirtualKeyCode;
use winit_input_helper::WinitInputHelper;

pub struct Joypad {
    pub rows: [u8; 2], // Button row, Direction row
    pub column_select: u8,
    pub interrupt: bool,
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            rows: [0x0F, 0x0F], // 1 = Released, 0 = Pressed
            column_select: 0,
            interrupt: false,
        }
    }

    pub fn update(&mut self, input: &WinitInputHelper) {
        let old_state = self.rows;

        // Reset (Release all)
        self.rows[0] = 0x0F; // A, B, Select, Start
        self.rows[1] = 0x0F; // Right, Left, Up, Down

        // Key Mapping
        // A = Z, B = X, Start = Enter, Select = Space
        if input.key_held(VirtualKeyCode::Z) {
            self.rows[0] &= !0x01;
        } // A
        if input.key_held(VirtualKeyCode::X) {
            self.rows[0] &= !0x02;
        } // B
        if input.key_held(VirtualKeyCode::Space) {
            self.rows[0] &= !0x04;
        } // Select
        if input.key_held(VirtualKeyCode::Return) {
            self.rows[0] &= !0x08;
        } // Start

        // Directions (Arrows)
        if input.key_held(VirtualKeyCode::Right) {
            self.rows[1] &= !0x01;
        }
        if input.key_held(VirtualKeyCode::Left) {
            self.rows[1] &= !0x02;
        }
        if input.key_held(VirtualKeyCode::Up) {
            self.rows[1] &= !0x04;
        }
        if input.key_held(VirtualKeyCode::Down) {
            self.rows[1] &= !0x08;
        }

        // Request interrupt if state changed
        if old_state != self.rows {
            self.interrupt = true;
        }
    }

    pub fn read(&self) -> u8 {
        let mut res = 0xCF; // Bits 6 and 7 always 1
        if (self.column_select & 0x10) == 0 {
            res &= self.rows[1];
        } // Directions
        if (self.column_select & 0x20) == 0 {
            res &= self.rows[0];
        } // Buttons
        res
    }

    pub fn write(&mut self, val: u8) {
        self.column_select = val & 0x30;
    }
}
