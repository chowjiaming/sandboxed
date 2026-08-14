//! Converts the world grid into an RGBA frame buffer for the browser.

use crate::world::{Cell, World, AIR_CELL};

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

fn warm(r: i16, g: i16, b: i16, heat: u8) -> (i16, i16, i16) {
    let t = heat as i16;
    (r + t / 2, g, b.saturating_sub(t / 3))
}

/// Write the whole world into `frame` (must be width*height*4 bytes).
pub fn draw(world: &World, frame: &mut [u8]) {
    for (i, cell) in world.cells().iter().enumerate() {
        let x = i % world.width;
        let y = i / world.width;
        let j = jitter(x, y);
        let h = world.heat()[i];
        match cell {
            Cell::Empty => {
                let ai = (y / AIR_CELL) * world.air_w + (x / AIR_CELL);
                let avx = world.vx()[ai];
                let avy = world.vy()[ai];
                put(frame, i, 12 + avx / 2, 12 - avx / 2, 18 + avy / 2);
            }
            Cell::Sand => {
                let (r, g, b) = warm(194 + j, 168 + j, 96 + j / 2, h);
                put(frame, i, r, g, b);
            }
            Cell::Water => {
                let (r, g, b) = warm(40, 90 + j, 200 + j, h);
                put(frame, i, r, g, b);
            }
            Cell::Stone => {
                let (r, g, b) = warm(110 + j / 2, 110 + j / 2, 118, h);
                put(frame, i, r, g, b);
            }
            Cell::Fire => {
                let t = world.ttl()[i] as i16;
                put(frame, i, 255, 32 + t * 4, 16);
            }
            Cell::Wood => {
                let (r, g, b) = warm(118 + j / 2, 72 + j / 2, 36, h);
                put(frame, i, r, g, b);
            }
            Cell::Steam => {
                let (r, g, b) = warm(216 + j / 2, 220 + j / 2, 232, h);
                put(frame, i, r, g, b);
            }
            Cell::FanRight => put(frame, i, 110, 120, 150),
            Cell::FanLeft => put(frame, i, 70, 100, 140),
            Cell::FanUp => put(frame, i, 90, 130, 160),
            Cell::FanDown => put(frame, i, 90, 100, 120),
            Cell::Gunpowder => {
                let (r, g, b) = warm(40 + j / 2, 36 + j / 2, 32, h);
                put(frame, i, r, g, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Cell, World};

    #[test]
    fn steam_pixels_are_pale() {
        let mut world = World::new(1, 1);
        world.paint(0, 0, Cell::Steam, 0);
        let mut frame = vec![0u8; 4];
        draw(&world, &mut frame);
        let (r, g, b) = (frame[0], frame[1], frame[2]);
        assert!(
            r > 180 && g > 180 && b > 180,
            "expected pale steam, got {r},{g},{b}"
        );
        assert_eq!(frame[3], 255);
    }

    #[test]
    fn hot_wood_is_warmer_than_cold_wood() {
        let mut cold = World::new(1, 1);
        cold.paint(0, 0, Cell::Wood, 0);
        let mut hot = World::new(1, 1);
        hot.paint(0, 0, Cell::Wood, 0);
        hot.add_heat_for_test(0, 0, 70);
        let mut fc = vec![0u8; 4];
        let mut fh = vec![0u8; 4];
        draw(&cold, &mut fc);
        draw(&hot, &mut fh);
        assert!(
            fh[0] > fc[0] || fh[2] < fc[2],
            "hot wood should be redder or less blue: cold={fc:?} hot={fh:?}"
        );
    }

    #[test]
    fn fire_pixels_are_orange_red() {
        let mut world = World::new(2, 1);
        world.paint(0, 0, Cell::Fire, 0);
        let mut frame = vec![0u8; 8];
        draw(&world, &mut frame);
        let (r, g, b) = (frame[0], frame[1], frame[2]);
        assert!(r > g && g > b, "expected orange/red, got {r},{g},{b}");
        assert_eq!(frame[3], 255);
    }

    #[test]
    fn empty_pixel_tints_when_fan_blows() {
        let still = World::new(8, 8);
        let mut blown = World::new(8, 8);
        blown.paint(0, 0, Cell::FanRight, 0);
        blown.step();
        let mut fs = vec![0u8; 8 * 8 * 4];
        let mut fb = vec![0u8; 8 * 8 * 4];
        draw(&still, &mut fs);
        draw(&blown, &mut fb);
        // Pixel (3, 0) is Empty in both; in blown it shares the fan's air cell.
        let o = (0 * 8 + 3) * 4;
        let (sr, sg, sb) = (fs[o], fs[o + 1], fs[o + 2]);
        let (br, bg, bb) = (fb[o], fb[o + 1], fb[o + 2]);
        assert!(
            br > sr && bg < sg,
            "FanRight should redden (+vx) and green-shift down (−vx): still=({sr},{sg},{sb}) blown=({br},{bg},{bb})"
        );
        assert_eq!(fb[o + 3], 255);
    }

    #[test]
    fn gunpowder_pixels_are_dark() {
        let mut world = World::new(1, 1);
        world.paint(0, 0, Cell::Gunpowder, 0);
        let mut frame = vec![0u8; 4];
        draw(&world, &mut frame);
        let (r, g, b) = (frame[0], frame[1], frame[2]);
        assert!(
            r < 80 && g < 80 && b < 80,
            "expected charcoal, got {r},{g},{b}"
        );
    }
}
