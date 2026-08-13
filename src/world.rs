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
    Fire = 4,
    Wood = 5,
}

impl Cell {
    pub fn from_u8(v: u8) -> Option<Cell> {
        match v {
            0 => Some(Cell::Empty),
            1 => Some(Cell::Sand),
            2 => Some(Cell::Water),
            3 => Some(Cell::Stone),
            4 => Some(Cell::Fire),
            5 => Some(Cell::Wood),
            _ => None,
        }
    }

    /// "Density" used for sinking behavior: sand sinks through water.
    fn density(self) -> u8 {
        match self {
            Cell::Empty => 0,
            Cell::Water => 1,
            Cell::Sand => 2,
            Cell::Fire => 0,
            Cell::Stone | Cell::Wood => 255, // immovable
        }
    }
}

/// Ticks a painted fire cell lives before it burns out.
const FIRE_LIFETIME: u8 = 48;
const CHUNK: usize = 16;

pub struct World {
    pub width: usize,
    pub height: usize,
    cells: Vec<Cell>,
    /// Per-cell remaining lifetime. Only `Fire` uses nonzero values.
    ttl: Vec<u8>,
    /// Scratch buffer marking cells that already moved this tick.
    moved: Vec<bool>,
    /// Tiny xorshift PRNG — no external crates needed in WASM.
    rng: u64,
    /// Frame counter; used to alternate scan direction and avoid bias.
    frame: u64,
    chunks_x: usize,
    chunks_y: usize,
    /// Chunks to process this tick.
    active: Vec<bool>,
    /// Chunks woken by motion this tick; becomes `active` after the step.
    next_active: Vec<bool>,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        let chunks_x = width.div_ceil(CHUNK);
        let chunks_y = height.div_ceil(CHUNK);
        let nchunks = chunks_x * chunks_y;
        Self {
            width,
            height,
            cells: vec![Cell::Empty; width * height],
            ttl: vec![0; width * height],
            moved: vec![false; width * height],
            rng: 0x853c_49e6_748f_ea9b,
            frame: 0,
            chunks_x,
            chunks_y,
            active: vec![false; nchunks],
            next_active: vec![false; nchunks],
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

    #[inline]
    pub fn ttl(&self) -> &[u8] {
        &self.ttl
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
                // Don't overwrite stone or wood with loose material (eraser can).
                let existing = self.get(px, py);
                if matches!(existing, Cell::Stone | Cell::Wood) && cell != Cell::Empty {
                    continue;
                }
                let i = self.idx(px, py);
                self.cells[i] = cell;
                self.ttl[i] = if cell == Cell::Fire { FIRE_LIFETIME } else { 0 };
                self.wake(px, py, false);
            }
        }
    }

    fn swap(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        let a = self.idx(x1, y1);
        let b = self.idx(x2, y2);
        self.cells.swap(a, b);
        self.ttl.swap(a, b);
        self.moved[b] = true;
        self.wake(x1, y1, true);
        self.wake(x2, y2, true);
    }

    /// Can `mover` displace `target`? Empty always; denser sinks
    /// through lighter fluids.
    fn can_displace(mover: Cell, target: Cell) -> bool {
        target == Cell::Empty || (target != Cell::Stone && mover.density() > target.density())
    }

    #[inline]
    fn chunk_i(&self, cx: usize, cy: usize) -> usize {
        cy * self.chunks_x + cx
    }

    fn wake_into(buf: &mut [bool], chunks_x: usize, chunks_y: usize, x: usize, y: usize) {
        let cx = x / CHUNK;
        let cy = y / CHUNK;
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let ncx = cx as isize + dx;
                let ncy = cy as isize + dy;
                if ncx < 0 || ncy < 0 {
                    continue;
                }
                let (ncx, ncy) = (ncx as usize, ncy as usize);
                if ncx >= chunks_x || ncy >= chunks_y {
                    continue;
                }
                buf[ncy * chunks_x + ncx] = true;
            }
        }
    }

    fn wake(&mut self, x: usize, y: usize, next: bool) {
        let chunks_x = self.chunks_x;
        let chunks_y = self.chunks_y;
        if next {
            Self::wake_into(&mut self.next_active, chunks_x, chunks_y, x, y);
        } else {
            Self::wake_into(&mut self.active, chunks_x, chunks_y, x, y);
        }
    }

    fn clear_moved_in_active_chunks(&mut self) {
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                if !self.active[self.chunk_i(cx, cy)] {
                    continue;
                }
                let x0 = cx * CHUNK;
                let x1 = ((cx + 1) * CHUNK).min(self.width);
                let y0 = cy * CHUNK;
                let y1 = ((cy + 1) * CHUNK).min(self.height);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = self.idx(x, y);
                        self.moved[i] = false;
                    }
                }
            }
        }
    }

    /// Advance the simulation by one tick.
    pub fn step(&mut self) {
        self.frame += 1;
        self.clear_moved_in_active_chunks();
        self.next_active.fill(false);

        let left_to_right = self.frame.is_multiple_of(2);
        for cy in (0..self.chunks_y).rev() {
            let mut any = false;
            for cx in 0..self.chunks_x {
                if self.active[self.chunk_i(cx, cy)] {
                    any = true;
                    break;
                }
            }
            if !any {
                continue;
            }
            let y0 = cy * CHUNK;
            let y1 = ((cy + 1) * CHUNK).min(self.height);
            for y in (y0..y1).rev() {
                for cxi in 0..self.chunks_x {
                    let cx = if left_to_right {
                        cxi
                    } else {
                        self.chunks_x - 1 - cxi
                    };
                    if !self.active[self.chunk_i(cx, cy)] {
                        continue;
                    }
                    let x0 = cx * CHUNK;
                    let x1 = ((cx + 1) * CHUNK).min(self.width);
                    let span = x1 - x0;
                    for xi in 0..span {
                        let x = if left_to_right { x0 + xi } else { x1 - 1 - xi };
                        let i = self.idx(x, y);
                        if self.moved[i] {
                            continue;
                        }
                        match self.cells[i] {
                            Cell::Sand => self.step_sand(x, y),
                            Cell::Water => self.step_water(x, y),
                            Cell::Fire => self.step_fire(x, y),
                            _ => {}
                        }
                    }
                }
            }
        }
        std::mem::swap(&mut self.active, &mut self.next_active);
        self.next_active.fill(false);
    }

    fn ignite(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        self.cells[i] = Cell::Fire;
        self.ttl[i] = FIRE_LIFETIME;
        self.moved[i] = true;
        self.wake(x, y, true);
    }

    fn has_adjacent_wood(&self, x: usize, y: usize) -> bool {
        self.for_each_cardinal(x, y, |cell| cell == Cell::Wood)
    }

    fn try_ignite_neighbors(&mut self, x: usize, y: usize) {
        const DIRS: [(isize, isize); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        for (dx, dy) in DIRS {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if nx >= self.width || ny >= self.height {
                continue;
            }
            if self.get(nx, ny) == Cell::Wood && self.next_rand().is_multiple_of(10) {
                self.ignite(nx, ny);
            }
        }
    }

    fn for_each_cardinal(&self, x: usize, y: usize, mut pred: impl FnMut(Cell) -> bool) -> bool {
        const DIRS: [(isize, isize); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        for (dx, dy) in DIRS {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if nx >= self.width || ny >= self.height {
                continue;
            }
            if pred(self.get(nx, ny)) {
                return true;
            }
        }
        false
    }

    fn extinguish(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        self.cells[i] = Cell::Empty;
        self.ttl[i] = 0;
        self.wake(x, y, true);
    }

    fn step_fire(&mut self, x: usize, y: usize) {
        self.wake(x, y, true);
        self.try_ignite_neighbors(x, y);
        let i = self.idx(x, y);
        let life = self.ttl[i];
        if life <= 1 {
            self.extinguish(x, y);
            return;
        }
        // Flicker: extra chance to die near the end of life.
        if life <= 4 && self.next_rand() & 7 == 0 {
            self.extinguish(x, y);
            return;
        }
        self.ttl[i] = life - 1;
        // Stay put while there is fuel so a flame doesn't float off a wall.
        if self.has_adjacent_wood(x, y) {
            return;
        }
        if y == 0 {
            return;
        }
        let above = y - 1;
        if self.try_move(x, y, x, above) {
            return;
        }
        let left_first = self.next_rand() & 1 == 0;
        let diagonals: [(isize, usize); 2] = [(-1, above), (1, above)];
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

    #[test]
    fn fire_does_not_fall() {
        let mut w = World::new(8, 8);
        w.paint(4, 3, Cell::Fire, 0);
        for _ in 0..32 {
            w.step();
            for y in 4..8 {
                for x in 0..8 {
                    assert_ne!(w.get(x, y), Cell::Fire, "fire fell to ({x},{y})");
                }
            }
        }
    }

    #[test]
    fn fire_eventually_disappears() {
        let mut w = World::new(8, 8);
        w.paint(4, 3, Cell::Fire, 0);
        assert_eq!(w.get(4, 3), Cell::Fire);
        for _ in 0..256 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Fire), 0);
    }

    #[test]
    fn fire_rises() {
        let mut w = World::new(8, 8);
        w.paint(4, 5, Cell::Fire, 0);
        w.step();
        assert_eq!(w.get(4, 4), Cell::Fire);
        assert_eq!(w.get(4, 5), Cell::Empty);
    }

    #[test]
    fn wood_is_static() {
        let mut w = World::new(8, 8);
        w.paint(4, 2, Cell::Wood, 0);
        for _ in 0..16 {
            w.step();
        }
        assert_eq!(w.get(4, 2), Cell::Wood);
    }

    #[test]
    fn wood_block_burns_within_n_ticks() {
        let mut w = World::new(8, 8);
        for x in 3..=5 {
            for y in 3..=5 {
                w.paint(x, y, Cell::Wood, 0);
            }
        }
        w.paint(2, 4, Cell::Fire, 0);
        assert_eq!(count(&w, Cell::Wood), 9);
        for _ in 0..2000 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Wood), 0, "wood block did not fully burn");
    }

    #[test]
    fn sparse_sand_grain_falls_across_chunks() {
        let mut w = World::new(64, 64);
        w.paint(32, 0, Cell::Sand, 0);
        for _ in 0..128 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Sand), 1);
        assert_eq!(w.get(32, 63), Cell::Sand);
    }

    #[test]
    #[ignore]
    fn bench_960x600_step_rate() {
        use std::time::Instant;
        const W: usize = 960;
        const H: usize = 600;
        const STEPS: u32 = 300;

        let mut sparse = World::new(W, H);
        sparse.paint(W / 2, 0, Cell::Sand, 0);
        let t = Instant::now();
        for _ in 0..STEPS {
            sparse.step();
        }
        let sparse_s = t.elapsed().as_secs_f64();

        let mut full = World::new(W, H);
        for x in (0..W).step_by(2) {
            for y in 0..H / 4 {
                full.paint(x, y, Cell::Sand, 0);
            }
        }
        let t = Instant::now();
        for _ in 0..STEPS {
            full.step();
        }
        let full_s = t.elapsed().as_secs_f64();

        println!(
            "960×600 native step rate ({STEPS} steps):\n  sparse 1 grain: {:.0} steps/s\n  dense top-quarter: {:.0} steps/s",
            STEPS as f64 / sparse_s,
            STEPS as f64 / full_s,
        );
    }
}
