// Ayuz - Unofficial Control Center for Asus Laptops
// Copyright (C) 2026 Guido Philipp
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see https://www.gnu.org/licenses/.

//! Pixel-shift orbit for OLED burn-in protection.
//!
//! The Windows MyASUS "OLED Shifter" moves static desktop content by one pixel
//! per timer tick along a fixed 9-position orbit around the origin and walks it
//! back and forth. This module reproduces that orbit; the caller decides which
//! UI element the offsets are applied to.

/// Offsets (dx, dy) in pixels relative to the unshifted position.
const ORBIT: [(i32, i32); 9] = [
    (0, 0),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Selectable shift intervals in seconds. The ASUS default is 200 s.
pub const INTERVALS_SECS: [u32; 5] = [60, 120, 200, 300, 600];

/// Index into [`INTERVALS_SECS`] used when nothing is configured.
pub const DEFAULT_INTERVAL_IDX: u32 = 2;

/// Seconds for a stored interval index, falling back to the default when out of range.
pub fn interval_secs(idx: u32) -> u64 {
    let idx = if (idx as usize) < INTERVALS_SECS.len() {
        idx
    } else {
        DEFAULT_INTERVAL_IDX
    };
    u64::from(INTERVALS_SECS[idx as usize])
}

/// Ping-pong walker over [`ORBIT`].
pub struct Orbit {
    idx: usize,
    forward: bool,
}

impl Orbit {
    pub fn new() -> Self {
        Self {
            idx: 0,
            forward: true,
        }
    }

    /// Current offset relative to the unshifted position.
    pub fn offset(&self) -> (i32, i32) {
        ORBIT[self.idx]
    }

    /// Moves to the next orbit position and returns the (dx, dy) delta to apply.
    pub fn advance(&mut self) -> (i32, i32) {
        let before = ORBIT[self.idx];
        if self.idx == ORBIT.len() - 1 {
            self.forward = false;
        } else if self.idx == 0 {
            self.forward = true;
        }
        if self.forward {
            self.idx += 1;
        } else {
            self.idx -= 1;
        }
        let after = ORBIT[self.idx];
        (after.0 - before.0, after.1 - before.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_at_most_one_pixel_per_axis() {
        let mut o = Orbit::new();
        for _ in 0..100 {
            let (dx, dy) = o.advance();
            assert!(dx.abs() <= 1 && dy.abs() <= 1);
        }
    }

    #[test]
    fn walks_back_to_origin_after_full_cycle() {
        let mut o = Orbit::new();
        for _ in 0..(ORBIT.len() - 1) * 2 {
            o.advance();
        }
        assert_eq!(o.offset(), (0, 0));
    }

    #[test]
    fn deltas_sum_to_offset() {
        let mut o = Orbit::new();
        let (mut x, mut y) = (0, 0);
        for _ in 0..37 {
            let (dx, dy) = o.advance();
            x += dx;
            y += dy;
            assert_eq!((x, y), o.offset());
        }
    }

    #[test]
    fn interval_falls_back_when_out_of_range() {
        assert_eq!(interval_secs(2), 200);
        assert_eq!(interval_secs(99), 200);
    }
}
