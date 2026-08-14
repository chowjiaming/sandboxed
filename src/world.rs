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
    Steam = 6,
    FanRight = 7,
    FanLeft = 8,
    FanUp = 9,
    FanDown = 10,
    Gunpowder = 11,
    Smoke = 12,
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
            6 => Some(Cell::Steam),
            7 => Some(Cell::FanRight),
            8 => Some(Cell::FanLeft),
            9 => Some(Cell::FanUp),
            10 => Some(Cell::FanDown),
            11 => Some(Cell::Gunpowder),
            12 => Some(Cell::Smoke),
            _ => None,
        }
    }

    /// "Density" used for sinking behavior: sand sinks through water.
    fn density(self) -> u8 {
        match self {
            Cell::Empty => 0,
            Cell::Water => 1,
            Cell::Gunpowder | Cell::Sand => 2,
            Cell::Fire | Cell::Steam | Cell::Smoke => 0,
            Cell::Stone
            | Cell::Wood
            | Cell::FanRight
            | Cell::FanLeft
            | Cell::FanUp
            | Cell::FanDown => 255,
        }
    }

    #[inline]
    fn is_fan(self) -> bool {
        matches!(
            self,
            Cell::FanRight | Cell::FanLeft | Cell::FanUp | Cell::FanDown
        )
    }

    #[inline]
    fn is_locked(self) -> bool {
        matches!(self, Cell::Stone | Cell::Wood) || self.is_fan()
    }

    #[inline]
    fn blocks_air(self) -> bool {
        matches!(self, Cell::Stone | Cell::Wood)
    }
}

/// Ticks a painted fire cell lives before it burns out.
const FIRE_LIFETIME: u8 = 48;
const FIRE_HEAT: u8 = 200;
const WOOD_IGNITE: u8 = 80;
const WATER_BOIL: u8 = 100;
const STEAM_CONDENSE: u8 = 40;
const STEAM_PAINT_HEAT: u8 = 120;
const SMOKE_PAINT_HEAT: u8 = 80;
const SMOKE_DISPERSE: u8 = 10;
const CHUNK: usize = 16;
pub(crate) const AIR_CELL: usize = 4;
const FAN_SPEED: i16 = 40;
const GAS_PUSH: i16 = 24;
const WATER_PUSH: i16 = 64;
const BOIL_PV: i16 = 80;
const GUN_PV: i16 = 400;
const GUN_IGNITE: u8 = 80;
const AIR_ADV: i32 = 32;
const AIR_FP: i32 = 256;

pub struct World {
    pub width: usize,
    pub height: usize,
    pub air_w: usize,
    pub air_h: usize,
    cells: Vec<Cell>,
    /// Per-cell remaining lifetime. Only `Fire` uses nonzero values.
    ttl: Vec<u8>,
    /// Per-cell heat for conduction and phase changes.
    heat: Vec<u8>,
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
    pv: Vec<i16>,
    vx: Vec<i16>,
    vy: Vec<i16>,
    pv_prev: Vec<i16>,
    vx_prev: Vec<i16>,
    vy_prev: Vec<i16>,
    /// Air cells that contain a locked pixel; velocity is zeroed after advection.
    blocked: Vec<bool>,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        let chunks_x = width.div_ceil(CHUNK);
        let chunks_y = height.div_ceil(CHUNK);
        let nchunks = chunks_x * chunks_y;
        let air_w = width.div_ceil(AIR_CELL);
        let air_h = height.div_ceil(AIR_CELL);
        let nair = air_w * air_h;
        Self {
            width,
            height,
            air_w,
            air_h,
            cells: vec![Cell::Empty; width * height],
            ttl: vec![0; width * height],
            heat: vec![0; width * height],
            moved: vec![false; width * height],
            rng: 0x853c_49e6_748f_ea9b,
            frame: 0,
            chunks_x,
            chunks_y,
            active: vec![false; nchunks],
            next_active: vec![false; nchunks],
            pv: vec![0; nair],
            vx: vec![0; nair],
            vy: vec![0; nair],
            pv_prev: vec![0; nair],
            vx_prev: vec![0; nair],
            vy_prev: vec![0; nair],
            blocked: vec![false; nair],
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

    #[inline]
    pub fn heat(&self) -> &[u8] {
        &self.heat
    }

    #[inline]
    fn air_idx(&self, ax: usize, ay: usize) -> usize {
        ay * self.air_w + ax
    }

    #[inline]
    fn air_idx_of(&self, x: usize, y: usize) -> usize {
        self.air_idx(x / AIR_CELL, y / AIR_CELL)
    }

    #[inline]
    #[cfg(test)]
    pub fn pv(&self) -> &[i16] {
        &self.pv
    }

    #[inline]
    pub fn vx(&self) -> &[i16] {
        &self.vx
    }

    #[inline]
    pub fn vy(&self) -> &[i16] {
        &self.vy
    }

    #[cfg(test)]
    pub(crate) fn air_idx_of_for_test(&self, x: usize, y: usize) -> usize {
        self.air_idx_of(x, y)
    }

    #[inline]
    fn clamp_i16(v: i32) -> i16 {
        v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }

    #[inline]
    fn damp_i16(v: i16, num: i32, den: i32) -> i16 {
        ((v as i32) * num / den) as i16
    }

    fn air_open(
        buf: &[i16],
        blocked: &[bool],
        air_w: usize,
        air_h: usize,
        ax: isize,
        ay: isize,
    ) -> i16 {
        if ax < 0 || ay < 0 {
            return 0;
        }
        let (ax, ay) = (ax as usize, ay as usize);
        if ax >= air_w || ay >= air_h {
            return 0;
        }
        let i = ay * air_w + ax;
        if blocked[i] {
            0
        } else {
            buf[i]
        }
    }

    fn fill_blocked(&mut self) {
        self.blocked.fill(false);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y).blocks_air() {
                    let i = self.air_idx_of(x, y);
                    self.blocked[i] = true;
                }
            }
        }
    }

    fn inject_fans(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                match self.get(x, y) {
                    Cell::FanRight => {
                        let i = self.air_idx_of(x, y);
                        self.vx[i] = FAN_SPEED;
                        self.vy[i] = 0;
                    }
                    Cell::FanLeft => {
                        let i = self.air_idx_of(x, y);
                        self.vx[i] = -FAN_SPEED;
                        self.vy[i] = 0;
                    }
                    Cell::FanUp => {
                        let i = self.air_idx_of(x, y);
                        self.vx[i] = 0;
                        self.vy[i] = -FAN_SPEED;
                    }
                    Cell::FanDown => {
                        let i = self.air_idx_of(x, y);
                        self.vx[i] = 0;
                        self.vy[i] = FAN_SPEED;
                    }
                    _ => {}
                }
            }
        }
    }

    fn blur_velocity(&mut self) {
        let aw = self.air_w;
        let ah = self.air_h;
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                let ax = ax as isize;
                let ay = ay as isize;
                let c = Self::air_open(&self.vx, &self.blocked, aw, ah, ax, ay) as i32;
                let l = Self::air_open(&self.vx, &self.blocked, aw, ah, ax - 1, ay) as i32;
                let r = Self::air_open(&self.vx, &self.blocked, aw, ah, ax + 1, ay) as i32;
                let u = Self::air_open(&self.vx, &self.blocked, aw, ah, ax, ay - 1) as i32;
                let d = Self::air_open(&self.vx, &self.blocked, aw, ah, ax, ay + 1) as i32;
                self.vx_prev[i] = Self::clamp_i16((4 * c + l + r + u + d) / 8);
                let c = Self::air_open(&self.vy, &self.blocked, aw, ah, ax, ay) as i32;
                let l = Self::air_open(&self.vy, &self.blocked, aw, ah, ax - 1, ay) as i32;
                let r = Self::air_open(&self.vy, &self.blocked, aw, ah, ax + 1, ay) as i32;
                let u = Self::air_open(&self.vy, &self.blocked, aw, ah, ax, ay - 1) as i32;
                let d = Self::air_open(&self.vy, &self.blocked, aw, ah, ax, ay + 1) as i32;
                self.vy_prev[i] = Self::clamp_i16((4 * c + l + r + u + d) / 8);
            }
        }
        std::mem::swap(&mut self.vx, &mut self.vx_prev);
        std::mem::swap(&mut self.vy, &mut self.vy_prev);
    }

    fn bilinear(
        buf: &[i16],
        blocked: &[bool],
        air_w: usize,
        air_h: usize,
        sx: i32,
        sy: i32,
    ) -> i16 {
        let x0 = sx.div_euclid(AIR_FP);
        let y0 = sy.div_euclid(AIR_FP);
        let fx = sx.rem_euclid(AIR_FP);
        let fy = sy.rem_euclid(AIR_FP);
        let v00 = Self::air_open(buf, blocked, air_w, air_h, x0 as isize, y0 as isize) as i32;
        let v10 = Self::air_open(buf, blocked, air_w, air_h, x0 as isize + 1, y0 as isize) as i32;
        let v01 = Self::air_open(buf, blocked, air_w, air_h, x0 as isize, y0 as isize + 1) as i32;
        let v11 =
            Self::air_open(buf, blocked, air_w, air_h, x0 as isize + 1, y0 as isize + 1) as i32;
        let a = v00 * (AIR_FP - fx) + v10 * fx;
        let b = v01 * (AIR_FP - fx) + v11 * fx;
        let c = a * (AIR_FP - fy) + b * fy;
        Self::clamp_i16(c / (AIR_FP * AIR_FP))
    }

    fn advect_velocity(&mut self) {
        let aw = self.air_w;
        let ah = self.air_h;
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                if self.blocked[i] {
                    self.vx_prev[i] = 0;
                    self.vy_prev[i] = 0;
                    continue;
                }
                let vx = self.vx[i] as i32;
                let vy = self.vy[i] as i32;
                let sx = ax as i32 * AIR_FP - vx * AIR_FP / AIR_ADV;
                let sy = ay as i32 * AIR_FP - vy * AIR_FP / AIR_ADV;
                self.vx_prev[i] = Self::bilinear(&self.vx, &self.blocked, aw, ah, sx, sy);
                self.vy_prev[i] = Self::bilinear(&self.vy, &self.blocked, aw, ah, sx, sy);
            }
        }
        std::mem::swap(&mut self.vx, &mut self.vx_prev);
        std::mem::swap(&mut self.vy, &mut self.vy_prev);
    }

    fn zero_blocked_velocity(&mut self) {
        for i in 0..self.blocked.len() {
            if self.blocked[i] {
                self.vx[i] = 0;
                self.vy[i] = 0;
            }
        }
    }

    fn step_air(&mut self) {
        self.fill_blocked();
        let aw = self.air_w;
        let ah = self.air_h;
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                let ax = ax as isize;
                let ay = ay as isize;
                let pv_l = Self::air_open(&self.pv, &self.blocked, aw, ah, ax - 1, ay) as i32;
                let pv_r = Self::air_open(&self.pv, &self.blocked, aw, ah, ax + 1, ay) as i32;
                let pv_u = Self::air_open(&self.pv, &self.blocked, aw, ah, ax, ay - 1) as i32;
                let pv_d = Self::air_open(&self.pv, &self.blocked, aw, ah, ax, ay + 1) as i32;
                let nvx = Self::clamp_i16(self.vx[i] as i32 + (pv_l - pv_r) / 4);
                let nvy = Self::clamp_i16(self.vy[i] as i32 + (pv_u - pv_d) / 4);
                self.vx_prev[i] = Self::damp_i16(nvx, 7, 8);
                self.vy_prev[i] = Self::damp_i16(nvy, 7, 8);
            }
        }
        std::mem::swap(&mut self.vx, &mut self.vx_prev);
        std::mem::swap(&mut self.vy, &mut self.vy_prev);
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                let ax = ax as isize;
                let ay = ay as isize;
                let vx_l = Self::air_open(&self.vx, &self.blocked, aw, ah, ax - 1, ay) as i32;
                let vx_r = Self::air_open(&self.vx, &self.blocked, aw, ah, ax + 1, ay) as i32;
                let vy_u = Self::air_open(&self.vy, &self.blocked, aw, ah, ax, ay - 1) as i32;
                let vy_d = Self::air_open(&self.vy, &self.blocked, aw, ah, ax, ay + 1) as i32;
                let npv = Self::clamp_i16(self.pv[i] as i32 + (vx_l - vx_r + vy_u - vy_d) / 4);
                self.pv_prev[i] = Self::damp_i16(npv, 15, 16);
            }
        }
        std::mem::swap(&mut self.pv, &mut self.pv_prev);
        self.blur_velocity();
        self.advect_velocity();
        self.zero_blocked_velocity();
        self.inject_fans();
        self.carry_jets();
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                if self.vx[i].abs() >= GAS_PUSH || self.vy[i].abs() >= GAS_PUSH {
                    self.wake(ax * AIR_CELL, ay * AIR_CELL, true);
                }
            }
        }
    }

    fn carry_jets(&mut self) {
        self.vx_prev.copy_from_slice(&self.vx);
        self.vy_prev.copy_from_slice(&self.vy);
        let aw = self.air_w;
        let ah = self.air_h;
        for ay in 0..ah {
            for ax in 0..aw {
                let i = self.air_idx(ax, ay);
                let vx = self.vx_prev[i];
                let vy = self.vy_prev[i];
                if vx.abs() < GAS_PUSH && vy.abs() < GAS_PUSH {
                    continue;
                }
                if vx.abs() >= vy.abs() {
                    let nx = ax as isize + vx.signum() as isize;
                    if nx < 0 || (nx as usize) >= aw {
                        continue;
                    }
                    let ni = self.air_idx(nx as usize, ay);
                    if self.blocked[ni] {
                        continue;
                    }
                    let carried = (vx as i32 * 3 / 4) as i16;
                    self.vx[ni] = if carried > 0 {
                        self.vx[ni].max(carried)
                    } else {
                        self.vx[ni].min(carried)
                    };
                } else {
                    let ny = ay as isize + vy.signum() as isize;
                    if ny < 0 || (ny as usize) >= ah {
                        continue;
                    }
                    let ni = self.air_idx(ax, ny as usize);
                    if self.blocked[ni] {
                        continue;
                    }
                    let carried = (vy as i32 * 3 / 4) as i16;
                    self.vy[ni] = if carried > 0 {
                        self.vy[ni].max(carried)
                    } else {
                        self.vy[ni].min(carried)
                    };
                }
            }
        }
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
                // Don't overwrite locked cells with loose material (eraser can).
                let existing = self.get(px, py);
                if existing.is_locked() && cell != Cell::Empty {
                    continue;
                }
                let i = self.idx(px, py);
                self.cells[i] = cell;
                self.ttl[i] = if cell == Cell::Fire { FIRE_LIFETIME } else { 0 };
                self.heat[i] = match cell {
                    Cell::Fire => FIRE_HEAT,
                    Cell::Steam => STEAM_PAINT_HEAT,
                    Cell::Smoke => SMOKE_PAINT_HEAT,
                    _ => 0,
                };
                self.wake(px, py, false);
            }
        }
    }

    fn swap(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        let a = self.idx(x1, y1);
        let b = self.idx(x2, y2);
        self.cells.swap(a, b);
        self.ttl.swap(a, b);
        self.heat.swap(a, b);
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
        self.step_air();
        // Air wakes land in next_active; the particle scan this tick uses active.
        for i in 0..self.active.len() {
            if self.next_active[i] {
                self.active[i] = true;
            }
        }
        self.clear_moved_in_active_chunks();

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
                            Cell::Gunpowder => self.step_gunpowder(x, y),
                            Cell::Water => self.step_water(x, y),
                            Cell::Fire => self.step_fire(x, y),
                            Cell::Steam => self.step_steam(x, y),
                            Cell::Smoke => self.step_smoke(x, y),
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
        self.heat[i] = FIRE_HEAT;
        self.moved[i] = true;
        self.wake(x, y, true);
    }

    fn add_pv(&mut self, x: usize, y: usize, amount: i16) {
        let i = self.air_idx_of(x, y);
        self.pv[i] = self.pv[i].saturating_add(amount);
        self.wake(x, y, true);
    }

    #[inline]
    fn add_heat(&mut self, x: usize, y: usize, amount: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = self.idx(x, y);
        if self.cells[i] == Cell::Empty {
            return;
        }
        self.heat[i] = self.heat[i].saturating_add(amount);
        self.wake(x, y, true);
        match self.cells[i] {
            Cell::Wood if self.heat[i] >= WOOD_IGNITE => self.ignite(x, y),
            Cell::Water if self.heat[i] >= WATER_BOIL => {
                self.cells[i] = Cell::Steam;
                self.ttl[i] = 0;
                self.add_pv(x, y, BOIL_PV);
            }
            _ => {}
        }
    }

    #[inline]
    fn conduct(&mut self, x: usize, y: usize) {
        const DIRS: [(isize, isize); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
        let i = self.idx(x, y);
        for (dx, dy) in DIRS {
            let src = self.heat[i];
            if src == 0 {
                return;
            }
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            if nx >= self.width || ny >= self.height {
                continue;
            }
            if self.get(nx, ny) == Cell::Empty {
                continue;
            }
            let ni = self.idx(nx, ny);
            let dst = self.heat[ni];
            if src <= dst {
                continue;
            }
            let give = ((src - dst) / 4).max(1);
            self.heat[i] = self.heat[i].saturating_sub(give);
            self.add_heat(nx, ny, give);
        }
    }

    fn has_adjacent_wood(&self, x: usize, y: usize) -> bool {
        self.for_each_cardinal(x, y, |cell| cell == Cell::Wood)
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

    fn first_cardinal(&self, x: usize, y: usize, want: Cell) -> Option<(usize, usize)> {
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
            if self.get(nx, ny) == want {
                return Some((nx, ny));
            }
        }
        None
    }

    fn extinguish(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        self.cells[i] = Cell::Empty;
        self.ttl[i] = 0;
        self.heat[i] = 0;
        self.wake(x, y, true);
    }

    fn become_smoke(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        self.cells[i] = Cell::Smoke;
        self.ttl[i] = 0;
        self.wake(x, y, true);
    }

    fn step_fire(&mut self, x: usize, y: usize) {
        self.wake(x, y, true);
        if let Some((wx, wy)) = self.first_cardinal(x, y, Cell::Water) {
            let wi = self.idx(wx, wy);
            let bump = WATER_BOIL.saturating_sub(self.heat[wi]);
            self.add_heat(wx, wy, bump);
            self.extinguish(x, y);
            return;
        }
        let i = self.idx(x, y);
        self.heat[i] = FIRE_HEAT;
        self.conduct(x, y);
        let life = self.ttl[i];
        if life <= 1 {
            self.become_smoke(x, y);
            return;
        }
        // Flicker: extra chance to die near the end of life.
        if life <= 4 && self.next_rand() & 7 == 0 {
            self.become_smoke(x, y);
            return;
        }
        self.ttl[i] = life - 1;
        // Stay put while there is fuel so a flame doesn't float off a wall.
        if self.has_adjacent_wood(x, y) {
            return;
        }
        if self.try_air_push(x, y, GAS_PUSH) {
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

    fn step_steam(&mut self, x: usize, y: usize) {
        self.wake(x, y, true);
        let i = self.idx(x, y);
        self.heat[i] = self.heat[i].saturating_sub(1);
        self.conduct(x, y);
        let i = self.idx(x, y);
        if self.cells[i] != Cell::Steam {
            return;
        }
        if self.heat[i] < STEAM_CONDENSE {
            self.cells[i] = Cell::Water;
            self.ttl[i] = 0;
            self.wake(x, y, true);
            return;
        }
        if self.try_air_push(x, y, GAS_PUSH) {
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

    fn step_smoke(&mut self, x: usize, y: usize) {
        self.wake(x, y, true);
        let i = self.idx(x, y);
        self.heat[i] = self.heat[i].saturating_sub(1);
        self.conduct(x, y);
        let i = self.idx(x, y);
        if self.cells[i] != Cell::Smoke {
            return;
        }
        if self.heat[i] < SMOKE_DISPERSE {
            self.cells[i] = Cell::Empty;
            self.heat[i] = 0;
            self.ttl[i] = 0;
            self.wake(x, y, true);
            return;
        }
        if self.try_air_push(x, y, GAS_PUSH) {
            return;
        }
        if !self.frame.is_multiple_of(2) {
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

    fn try_air_push(&mut self, x: usize, y: usize, threshold: i16) -> bool {
        let i = self.air_idx_of(x, y);
        let vx = self.vx[i];
        let vy = self.vy[i];
        let (dx, dy) = if vx.abs() >= vy.abs() {
            if vx.abs() < threshold {
                return false;
            }
            (vx.signum() as isize, 0)
        } else {
            if vy.abs() < threshold {
                return false;
            }
            (0, vy.signum() as isize)
        };
        let nx = x as isize + dx;
        let ny = y as isize + dy;
        if nx < 0 || ny < 0 {
            return false;
        }
        self.try_move(x, y, nx as usize, ny as usize)
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

    fn explode_gunpowder(&mut self, x: usize, y: usize) {
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let nx = x as isize + dx;
                let ny = y as isize + dy;
                if nx < 0 || ny < 0 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                if nx >= self.width || ny >= self.height {
                    continue;
                }
                let cell = self.get(nx, ny);
                if cell == Cell::Stone || cell.is_fan() {
                    continue;
                }
                self.ignite(nx, ny);
            }
        }
        self.add_pv(x, y, GUN_PV);
    }

    fn step_gunpowder(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        if self.heat[i] >= GUN_IGNITE || self.first_cardinal(x, y, Cell::Fire).is_some() {
            self.explode_gunpowder(x, y);
            return;
        }
        self.step_sand(x, y);
    }

    #[cfg(test)]
    pub(crate) fn add_heat_for_test(&mut self, x: usize, y: usize, amount: u8) {
        self.add_heat(x, y, amount);
    }

    #[cfg(test)]
    pub(crate) fn swap_for_test(&mut self, x1: usize, y1: usize, x2: usize, y2: usize) {
        self.swap(x1, y1, x2, y2);
    }

    fn step_water(&mut self, x: usize, y: usize) {
        let i = self.idx(x, y);
        if self.heat[i] > 0 {
            self.conduct(x, y);
            if self.get(x, y) != Cell::Water {
                return;
            }
        }
        if let Some((fx, fy)) = self.first_cardinal(x, y, Cell::Fire) {
            let bump = WATER_BOIL.saturating_sub(self.heat[i]);
            self.add_heat(x, y, bump);
            self.extinguish(fx, fy);
            return;
        }
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
        self.try_air_push(x, y, WATER_PUSH);
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
    fn fire_burns_to_smoke_then_empty() {
        let mut w = World::new(8, 8);
        w.paint(4, 3, Cell::Fire, 0);
        let mut saw_smoke = false;
        for _ in 0..400 {
            w.step();
            if count(&w, Cell::Smoke) > 0 {
                saw_smoke = true;
            }
            if saw_smoke && count(&w, Cell::Fire) == 0 && count(&w, Cell::Smoke) == 0 {
                break;
            }
        }
        assert!(saw_smoke, "isolated fire should leave smoke");
        assert_eq!(count(&w, Cell::Fire), 0);
        assert_eq!(count(&w, Cell::Smoke), 0);
    }

    #[test]
    fn water_quench_does_not_spawn_smoke() {
        let mut w = World::new(8, 8);
        w.paint(3, 4, Cell::Fire, 0);
        w.paint(4, 4, Cell::Water, 0);
        for _ in 0..16 {
            w.step();
            assert_eq!(count(&w, Cell::Smoke), 0);
        }
    }

    #[test]
    fn smoke_rises_slower_than_steam() {
        let mut steam = World::new(8, 16);
        let mut smoke = World::new(8, 16);
        steam.paint(4, 14, Cell::Steam, 0);
        smoke.paint(4, 14, Cell::Smoke, 0);
        for _ in 0..8 {
            steam.step();
            smoke.step();
        }
        let steam_y = (0..16)
            .find(|&y| (0..8).any(|x| steam.get(x, y) == Cell::Steam))
            .expect("steam still present");
        let smoke_y = (0..16)
            .find(|&y| (0..8).any(|x| smoke.get(x, y) == Cell::Smoke))
            .expect("smoke still present");
        assert!(
            smoke_y > steam_y,
            "smoke should lag steam: smoke_y={smoke_y} steam_y={steam_y}"
        );
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
    fn wood_ignites_when_heat_reaches_threshold() {
        let mut w = World::new(8, 8);
        w.paint(4, 4, Cell::Wood, 0);
        w.paint(3, 4, Cell::Fire, 0);
        for _ in 0..8 {
            w.step();
        }
        assert_ne!(
            w.get(4, 4),
            Cell::Wood,
            "wood did not ignite from conducted heat"
        );
        assert_eq!(count(&w, Cell::Wood), 0);
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
    fn heating_wood_stays_active_across_chunks() {
        let mut w = World::new(64, 64);
        w.paint(40, 40, Cell::Wood, 0);
        w.paint(39, 40, Cell::Fire, 0);
        w.step();
        let hi = 40 * 64 + 40;
        assert!(
            w.heat()[hi] > 0 || w.get(40, 40) == Cell::Fire,
            "wood in a sparse world did not receive heat (chunk slept)"
        );
        for _ in 0..8 {
            w.step();
        }
        assert_ne!(w.get(40, 40), Cell::Wood);
    }

    #[test]
    fn water_at_boil_becomes_steam() {
        let mut w = World::new(4, 4);
        w.paint(1, 1, Cell::Water, 0);
        w.add_heat_for_test(1, 1, WATER_BOIL);
        assert_eq!(w.get(1, 1), Cell::Steam);
        assert!(w.heat()[1 * 4 + 1] >= WATER_BOIL);
    }

    #[test]
    fn boil_adds_pressure() {
        let mut w = World::new(4, 4);
        w.paint(1, 1, Cell::Water, 0);
        let i = w.air_idx_of_for_test(1, 1);
        let before = w.pv()[i];
        w.add_heat_for_test(1, 1, WATER_BOIL);
        assert_eq!(w.get(1, 1), Cell::Steam);
        assert!(
            w.pv()[i] >= before + BOIL_PV,
            "boil should add pv: before={before} after={}",
            w.pv()[i]
        );
    }

    #[test]
    fn gunpowder_explodes_from_cardinal_fire() {
        let mut w = World::new(8, 8);
        w.paint(3, 3, Cell::Gunpowder, 0);
        w.paint(3, 2, Cell::Fire, 0);
        w.paint(2, 2, Cell::Stone, 0);
        w.paint(2, 3, Cell::FanRight, 0);
        w.step();
        assert_eq!(w.get(2, 2), Cell::Stone, "stone in the 3×3 must remain");
        assert_eq!(w.get(2, 3), Cell::FanRight, "fan in the 3×3 must remain");
        assert_eq!(w.get(3, 3), Cell::Fire);
        assert_eq!(w.get(4, 3), Cell::Fire);
        assert_eq!(w.get(3, 4), Cell::Fire);
        let i = w.air_idx_of_for_test(3, 3);
        assert!(
            w.pv()[i] >= GUN_PV,
            "explode is after this tick's Euler, pv should still be >= GUN_PV, got {}",
            w.pv()[i]
        );
    }

    #[test]
    fn gunpowder_explodes_from_heat_without_fire() {
        let mut w = World::new(8, 8);
        w.paint(3, 3, Cell::Gunpowder, 0);
        w.add_heat_for_test(3, 3, GUN_IGNITE);
        w.step();
        assert_eq!(w.get(3, 3), Cell::Fire);
        assert_eq!(
            count(&w, Cell::Fire),
            9,
            "3×3 empty neighborhood becomes fire"
        );
    }

    #[test]
    fn fan_velocity_stays_bounded_over_long_run() {
        let mut w = World::new(32, 32);
        w.paint(0, 0, Cell::FanRight, 0);
        for _ in 0..600 {
            w.step();
        }
        let peak = w
            .vx()
            .iter()
            .chain(w.vy().iter())
            .map(|v| v.abs())
            .max()
            .unwrap();
        assert!(
            peak <= FAN_SPEED,
            "air |v| ran away: peak={peak} FAN_SPEED={FAN_SPEED}"
        );
    }

    #[test]
    fn gunpowder_pressure_impulse_decays() {
        let mut w = World::new(32, 32);
        w.paint(8, 8, Cell::Gunpowder, 0);
        w.paint(8, 7, Cell::Fire, 0);
        w.step(); // explodes this tick; pv >= GUN_PV
        for _ in 0..600 {
            w.step();
        }
        let peak_pv = w.pv().iter().map(|v| v.abs()).max().unwrap();
        let peak_v = w
            .vx()
            .iter()
            .chain(w.vy().iter())
            .map(|v| v.abs())
            .max()
            .unwrap();
        assert!(
            peak_pv < 8 && peak_v < 8,
            "shockwave did not decay: peak_pv={peak_pv} peak_v={peak_v}"
        );
    }

    #[test]
    fn gunpowder_shockwave_displaces_nearby_water() {
        let mut w = World::new(32, 32);
        w.paint(10, 6, Cell::Water, 0);
        w.paint(10, 12, Cell::Gunpowder, 0);
        w.paint(10, 11, Cell::Fire, 0);
        let wi = w.air_idx_of_for_test(10, 6);
        let mut peak = 0i16;
        for _ in 0..16 {
            w.step();
            peak = peak.max(w.vx()[wi].abs()).max(w.vy()[wi].abs());
        }
        assert!(
            peak >= GAS_PUSH,
            "blast should raise |v| at the water's air cell, peak={peak}"
        );
    }

    #[test]
    fn gunpowder_falls_like_sand() {
        let mut w = World::new(8, 8);
        w.paint(4, 0, Cell::Gunpowder, 0);
        for _ in 0..16 {
            w.step();
        }
        assert_eq!(w.get(4, 7), Cell::Gunpowder);
    }

    #[test]
    fn fire_touching_water_extinguishes_and_boils() {
        let mut w = World::new(8, 8);
        w.paint(4, 4, Cell::Water, 0);
        w.paint(3, 4, Cell::Fire, 0);
        w.step();
        assert_eq!(count(&w, Cell::Fire), 0, "fire survived contact with water");
        assert_eq!(count(&w, Cell::Steam), 1);
        let steam_i = w
            .cells()
            .iter()
            .position(|&c| c == Cell::Steam)
            .expect("steam");
        assert!(w.heat()[steam_i] >= WATER_BOIL);
    }

    #[test]
    fn painted_steam_has_heat_120() {
        let mut w = World::new(4, 4);
        w.paint(1, 1, Cell::Steam, 0);
        assert_eq!(w.get(1, 1), Cell::Steam);
        assert_eq!(w.heat()[1 * 4 + 1], 120);
    }

    #[test]
    fn empty_cells_have_zero_heat() {
        let w = World::new(2, 2);
        assert!(w.heat().iter().all(|&h| h == 0));
        assert!(w.ttl().iter().all(|&t| t == 0));
    }

    #[test]
    fn swap_moves_heat_with_the_cell() {
        let mut w = World::new(4, 4);
        w.paint(0, 0, Cell::Steam, 0);
        assert_eq!(w.heat()[0], 120);
        w.swap_for_test(0, 0, 1, 0);
        assert_eq!(w.get(1, 0), Cell::Steam);
        assert_eq!(w.heat()[1], 120);
        assert_eq!(w.get(0, 0), Cell::Empty);
        assert_eq!(w.heat()[0], 0);
    }

    #[test]
    fn from_u8_maps_steam() {
        assert_eq!(Cell::from_u8(6), Some(Cell::Steam));
    }

    #[test]
    fn from_u8_maps_fans_and_gunpowder() {
        assert_eq!(Cell::from_u8(7), Some(Cell::FanRight));
        assert_eq!(Cell::from_u8(8), Some(Cell::FanLeft));
        assert_eq!(Cell::from_u8(9), Some(Cell::FanUp));
        assert_eq!(Cell::from_u8(10), Some(Cell::FanDown));
        assert_eq!(Cell::from_u8(11), Some(Cell::Gunpowder));
        assert_eq!(Cell::from_u8(12), Some(Cell::Smoke));
        assert_eq!(Cell::from_u8(13), None);
    }

    #[test]
    fn paint_cannot_overwrite_fan_except_eraser() {
        let mut w = World::new(4, 4);
        w.paint(1, 1, Cell::FanRight, 0);
        w.paint(1, 1, Cell::Sand, 0);
        assert_eq!(w.get(1, 1), Cell::FanRight);
        w.paint(1, 1, Cell::Empty, 0);
        assert_eq!(w.get(1, 1), Cell::Empty);
    }

    #[test]
    fn air_buffers_match_grid() {
        let w = World::new(320, 200);
        assert_eq!(w.air_w, 80);
        assert_eq!(w.air_h, 50);
        assert_eq!(w.pv().len(), 80 * 50);
        assert_eq!(w.vx().len(), w.pv().len());
        assert_eq!(w.vy().len(), w.pv().len());
    }

    #[test]
    fn fan_injects_velocity_on_facing_axis() {
        let mut w = World::new(8, 8);
        w.paint(0, 0, Cell::FanRight, 0);
        w.step();
        let i = w.air_idx_of_for_test(0, 0);
        assert!(w.vx()[i] > 0, "expected +vx, got {}", w.vx()[i]);
        assert_eq!(w.vy()[i], 0);
        assert_eq!(w.get(0, 0), Cell::FanRight);
    }

    #[test]
    fn idle_fan_still_blows_after_chunks_sleep() {
        let mut w = World::new(8, 8);
        w.paint(0, 0, Cell::FanDown, 0);
        for _ in 0..8 {
            w.step();
        }
        let i = w.air_idx_of_for_test(0, 0);
        assert!(
            w.vy()[i] > 0,
            "idle fan lost vy (inject skipped sleeping chunk?): {}",
            w.vy()[i]
        );
    }

    #[test]
    fn orthogonal_fans_last_in_scan_order_wins() {
        let mut w = World::new(8, 8);
        w.paint(0, 0, Cell::FanRight, 0);
        w.paint(1, 0, Cell::FanDown, 0);
        w.step();
        let i = w.air_idx_of_for_test(0, 0);
        assert_eq!(w.vx()[i], 0, "FanDown should zero vx from earlier FanRight");
        assert!(
            w.vy()[i] > 0,
            "FanDown should inject +vy, got {}",
            w.vy()[i]
        );
    }

    #[test]
    fn steam_in_front_of_fan_moves_right() {
        let mut blown = World::new(24, 8);
        blown.paint(0, 4, Cell::FanRight, 0);
        blown.paint(1, 4, Cell::Steam, 0);
        let mut still = World::new(24, 8);
        still.paint(1, 4, Cell::Steam, 0);
        for _ in 0..16 {
            blown.step();
            still.step();
        }
        let blown_x = (0..24)
            .find(|&x| (0..8).any(|y| blown.get(x, y) == Cell::Steam))
            .expect("steam still in blown world");
        let still_x = (0..24)
            .find(|&x| (0..8).any(|y| still.get(x, y) == Cell::Steam))
            .expect("steam still in control world");
        assert!(
            blown_x > still_x,
            "fan should push steam right: blown_x={blown_x} still_x={still_x}"
        );
    }

    #[test]
    fn stone_wall_blocks_shockwave() {
        let mut open = World::new(32, 16);
        open.paint(2, 8, Cell::Gunpowder, 0);
        open.paint(2, 7, Cell::Fire, 0);
        let mut walled = World::new(32, 16);
        for y in 0..16 {
            walled.paint(12, y, Cell::Stone, 0);
        }
        walled.paint(2, 8, Cell::Gunpowder, 0);
        walled.paint(2, 7, Cell::Fire, 0);
        for _ in 0..24 {
            open.step();
            walled.step();
        }
        let far = open.air_idx_of_for_test(28, 8);
        assert!(
            walled.pv()[far].abs() < 8,
            "shockwave tunneled through stone: pv={}",
            walled.pv()[far]
        );
        assert!(
            walled.pv()[far].abs() < open.pv()[far].abs().max(1),
            "walled far pv should be below open far pv: walled={} open={}",
            walled.pv()[far],
            open.pv()[far]
        );
    }

    #[test]
    fn steam_in_next_air_cell_of_fan_moves_right() {
        let mut blown = World::new(32, 12);
        blown.paint(0, 6, Cell::FanRight, 0);
        blown.paint(5, 6, Cell::Steam, 0);
        let mut still = World::new(32, 12);
        still.paint(5, 6, Cell::Steam, 0);
        for _ in 0..32 {
            blown.step();
            still.step();
        }
        let blown_x = (0..32)
            .find(|&x| (0..12).any(|y| blown.get(x, y) == Cell::Steam))
            .expect("steam still in blown world");
        let still_x = (0..32)
            .find(|&x| (0..12).any(|y| still.get(x, y) == Cell::Steam))
            .expect("steam still in control world");
        assert!(
            blown_x > still_x,
            "advection should carry fan wind into the next 4×4: blown_x={blown_x} still_x={still_x}"
        );
    }

    #[test]
    fn sand_next_to_fan_does_not_blow_away() {
        let mut w = World::new(8, 8);
        w.paint(0, 6, Cell::FanRight, 0);
        for x in 0..8 {
            w.paint(x, 7, Cell::Stone, 0);
        }
        w.paint(1, 6, Cell::Sand, 0);
        for _ in 0..16 {
            w.step();
        }
        assert_eq!(w.get(1, 6), Cell::Sand);
        for x in 2..8 {
            for y in 0..7 {
                assert_ne!(w.get(x, y), Cell::Sand, "sand blown to ({x},{y})");
            }
        }
    }

    #[test]
    fn steam_rises() {
        let mut w = World::new(8, 8);
        w.paint(4, 5, Cell::Steam, 0);
        w.step();
        assert_eq!(w.get(4, 4), Cell::Steam);
        assert_eq!(w.get(4, 5), Cell::Empty);
        assert_eq!(w.heat()[4 * 8 + 4], 119);
        assert_eq!(w.heat()[4 * 8 + 5], 0);
    }

    #[test]
    fn steam_condenses_when_heat_drops() {
        let mut w = World::new(4, 4);
        w.paint(1, 1, Cell::Steam, 0);
        // 120 -> 39 takes 81 cools; isolated steam only loses 1/tick.
        for _ in 0..81 {
            w.step();
        }
        assert_eq!(count(&w, Cell::Steam), 0);
        assert_eq!(count(&w, Cell::Water), 1);
        let wi = w
            .cells()
            .iter()
            .position(|&c| c == Cell::Water)
            .expect("water");
        assert!(w.heat()[wi] < STEAM_CONDENSE);
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
