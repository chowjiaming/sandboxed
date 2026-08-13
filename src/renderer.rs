//! Converts the world grid into an RGBA frame buffer for the browser.

use crate::world::{Cell, World};

/// Deterministic per-pixel color jitter so large piles look textured
/// instead of flat. Uses a cheap integer hash of (x, y).
fn jitter(x: usize, y: usize) -> i16 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9);
    h ^= (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    ((h % 21) as i16) - 10 // -10..=10
}

fn put(frame: &mut [u8], i: usize, r: i16, g: i16, b: i16) {
    let o = i * 4;
    frame[o] = r.clamp(0, 255) as u8;
    frame[o + 1] = g.clamp(0, 255) as u8;
    frame[o + 2] = b.clamp(0, 255) as u8;
    frame[o + 3] = 255;
}

/// Write the whole world into `frame` (must be width*height*4 bytes).
pub fn draw(world: &World, frame: &mut [u8]) {
    for (i, cell) in world.cells().iter().enumerate() {
        let x = i % world.width;
        let y = i / world.width;
        let j = jitter(x, y);
        match cell {
            Cell::Empty => put(frame, i, 12, 12, 18),
            Cell::Sand => put(frame, i, 194 + j, 168 + j, 96 + j / 2),
            Cell::Water => put(frame, i, 40, 90 + j, 200 + j),
            Cell::Stone => put(frame, i, 110 + j / 2, 110 + j / 2, 118),
        }
    }
}
