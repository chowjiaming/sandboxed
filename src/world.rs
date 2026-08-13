//! Core simulation: grid storage, cell types, and per-tick rules.

/// The material occupying a single grid cell.
///
/// `repr(u8)` keeps the grid as a flat, cache-friendly byte buffer
/// and makes it trivial to expose to JS later if needed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cell {
    Empty = 0,
    Sand = 1,
    Water = 2,
    Stone = 3,
}

impl Cell {
    pub fn from_u8(v: u8) -> Option<Cell> {
        match v {
            0 => Some(Cell::Empty),
            1 => Some(Cell::Sand),
            2 => Some(Cell::Water),
            3 => Some(Cell::Stone),
            _ => None,
        }
    }

    /// "Density" used for sinking behavior: sand sinks through water.
    fn density(self) -> u8 {
        match self {
            Cell::Empty => 0,
            Cell::Water => 1,
            Cell::Sand => 2,
            Cell::Stone => 255, // immovable
        }
    }
}

pub struct World {
    pub width: usize,
    pub height: usize,
    cells: Vec<Cell>,
    /// Scratch buffer marking cells that already moved this tick.
    moved: Vec<bool>,
    /// Tiny xorshift PRNG — no external crates needed in WASM.
    rng: u64,
    /// Frame counter; used to alternate scan direction and avoid bias.
    frame: u64,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::Empty; width * height],
            moved: vec![false; width * height],
            rng: 0x853c_49e6_748f_ea9b,
            frame: 0,
        }
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Cell {
        self.cells[self.idx(x, y)]
    }

    #[inline]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Paint a circular brush of `cell` centered at (x, y).
    pub fn paint(&mut self, x: usize, y: usize, cell: Cell, radius: usize) {
        let r = radius as isize;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let px = x as isize + dx;
                let py = y as isize + dy;
                if px < 0 || py < 0 {
                    continue;
                }
                let (px, py) = (px as usize, py as usize);
                if px >= self.width || py >= self.height {
                    continue;
                }
                // Don't overwrite stone with loose material (eraser can).
                let existing = self.get(px, py);
                if existing == Cell::Stone && cell != Cell::Empty {
                    continue;
                }
                let i = self.idx(px, py);
                self.cells[i] = cell;
            }
        }
    }

    fn swap(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        let a = self.idx(x1, y1);
        let b = self.idx(x2, y2);
        self.cells.swap(a, b);
        self.moved[b] = true;
    }

    /// Can `mover` displace `target`? Empty always; denser sinks
    /// through lighter fluids.
    fn can_displace(mover: Cell, target: Cell) -> bool {
        target == Cell::Empty || (target != Cell::Stone && mover.density() > target.density())
    }

    /// Advance the simulation by one tick.
    pub fn step(&mut self) {
        self.moved.fill(false);
        self.frame += 1;

        // Bottom-to-top so falling particles don't chain-move in
        // one tick. Include the last row so floor liquids can slide.
        for y in (0..self.height).rev() {
            // Alternate horizontal scan direction each frame to
            // prevent visible directional drift.
            let left_to_right = self.frame.is_multiple_of(2);
            for xi in 0..self.width {
                let x = if left_to_right {
                    xi
                } else {
                    self.width - 1 - xi
                };
                let i = self.idx(x, y);
                if self.moved[i] {
                    continue;
                }
                match self.cells[i] {
                    Cell::Sand => self.step_sand(x, y),
                    Cell::Water => self.step_water(x, y),
                    _ => {}
                }
            }
        }
    }

    fn try_move(&mut self, x: usize, y: usize, nx: usize, ny: usize) -> bool {
        let mover = self.get(x, y);
        if nx >= self.width || ny >= self.height {
            return false;
        }
        if Self::can_displace(mover, self.get(nx, ny)) {
            self.swap(x, y, nx, ny);
            true
        } else {
            false
        }
    }

    fn step_sand(&mut self, x: usize, y: usize) {
        let below = y + 1;
        if below >= self.height {
            return;
        }
        // Straight down first (sand sinks through water via density).
        if self.try_move(x, y, x, below) {
            return;
        }
        // Then a random diagonal.
        let left_first = self.next_rand() & 1 == 0;
        let diagonals: [(isize, usize); 2] = [(-1, below), (1, below)];
        for k in 0..2 {
            let (dx, ny) = diagonals[if left_first { k } else { 1 - k }];
            let nx = x as isize + dx;
            if nx < 0 {
                continue;
            }
            if self.try_move(x, y, nx as usize, ny) {
                return;
            }
        }
    }

    fn step_water(&mut self, x: usize, y: usize) {
        let below = y + 1;
        if below < self.height && self.try_move(x, y, x, below) {
            return;
        }
        let left_first = self.next_rand() & 1 == 0;
        // Diagonals, then horizontal slide (water spreads).
        let moves: [(isize, isize); 4] = [(-1, 1), (1, 1), (-1, 0), (1, 0)];
        for k in 0..4 {
            let (dx, dy) = moves[if left_first { k } else { 3 - k }];
            let nx = x as isize + dx;
            let ny = (y as isize + dy) as usize;
            if nx < 0 {
                continue;
            }
            if self.try_move(x, y, nx as usize, ny) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sand_falls_to_bottom() {
        let mut w = World::new(8, 8);
        w.paint(4, 0, Cell::Sand, 0);
        for _ in 0..16 {
            w.step();
        }
        assert_eq!(w.get(4, 7), Cell::Sand);
    }

    #[test]
    fn stone_never_moves() {
        let mut w = World::new(8, 8);
        w.paint(4, 2, Cell::Stone, 0);
        for _ in 0..16 {
            w.step();
        }
        assert_eq!(w.get(4, 2), Cell::Stone);
    }

    #[test]
    fn sand_sinks_through_water() {
        let mut w = World::new(4, 4);
        w.paint(2, 0, Cell::Sand, 0);
        w.paint(2, 1, Cell::Water, 0);
        for _ in 0..8 {
            w.step();
        }
        assert_eq!(w.get(2, 3), Cell::Sand);
    }

    fn count(w: &World, cell: Cell) -> usize {
        w.cells().iter().filter(|&&c| c == cell).count()
    }

    #[test]
    fn water_on_floor_spreads_horizontally() {
        let mut w = World::new(8, 4);
        w.paint(0, 3, Cell::Water, 0);
        for _ in 0..32 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Water), 1);
        assert_ne!(
            w.get(0, 3),
            Cell::Water,
            "water stuck at spawn on the world floor"
        );
        let on_floor = (0..8).filter(|&x| w.get(x, 3) == Cell::Water).count();
        assert_eq!(on_floor, 1);
    }

    #[test]
    fn water_levels_out_in_a_container() {
        let mut w = World::new(9, 6);
        for y in 1..=4 {
            w.paint(1, y, Cell::Stone, 0);
            w.paint(7, y, Cell::Stone, 0);
        }
        for x in 1..=7 {
            w.paint(x, 4, Cell::Stone, 0);
        }
        for y in 1..=3 {
            w.paint(2, y, Cell::Water, 0);
            w.paint(3, y, Cell::Water, 0);
        }
        let water0 = count(&w, Cell::Water);
        for _ in 0..200 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Water), water0);

        let mut heights = [0usize; 9];
        for y in 0..4 {
            for x in 0..9 {
                if w.get(x, y) == Cell::Water {
                    assert!((2..=6).contains(&x), "water escaped at ({x},{y})");
                    heights[x] += 1;
                }
            }
        }
        let inner = &heights[2..=6];
        let max = *inner.iter().max().unwrap();
        let min = *inner.iter().min().unwrap();
        assert!(
            max - min <= 1,
            "water column heights {inner:?} are not level"
        );
        assert!(min >= 1, "container did not fill across the floor");
    }

    #[test]
    fn sand_forms_a_45_degree_pile() {
        let mut w = World::new(15, 10);
        let floor = 9;
        let cx = 7;
        for x in 0..15 {
            w.paint(x, floor, Cell::Stone, 0);
        }
        for y in 0..8 {
            w.paint(cx, y, Cell::Sand, 0);
        }
        for _ in 0..200 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Sand), 8);

        let mut height = [0usize; 15];
        for y in 0..floor {
            for x in 0..15 {
                if w.get(x, y) == Cell::Sand {
                    height[x] += 1;
                }
            }
        }
        let occupied = height.iter().filter(|&&h| h > 0).count();
        assert!(occupied > 1, "sand stayed in a column instead of piling");
        for x in 0..14 {
            let dh = height[x].abs_diff(height[x + 1]);
            assert!(
                dh <= 1,
                "slope steeper than 45° between x={x} and x={}: {height:?}",
                x + 1
            );
        }
    }

    #[test]
    fn sand_has_no_left_right_drift_bias() {
        let mut w = World::new(21, 12);
        let cx = 10;
        let floor = 11;
        for x in 0..21 {
            w.paint(x, floor, Cell::Stone, 0);
        }
        for y in 0..8 {
            w.paint(cx, y, Cell::Sand, 0);
        }
        for _ in 0..1000 {
            w.step();
        }
        let mut left = 0usize;
        let mut right = 0usize;
        for y in 0..floor {
            for x in 0..21 {
                if w.get(x, y) != Cell::Sand {
                    continue;
                }
                match x.cmp(&cx) {
                    std::cmp::Ordering::Less => left += 1,
                    std::cmp::Ordering::Greater => right += 1,
                    std::cmp::Ordering::Equal => {}
                }
            }
        }
        let diff = left.abs_diff(right);
        assert!(
            diff <= 1,
            "left={left} right={right} drifted after 1000 ticks"
        );
    }
}
