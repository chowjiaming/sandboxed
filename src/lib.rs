//! WASM entry point. Keep this file thin: all logic lives in
//! `world` and `renderer`, which stay pure Rust and testable.

mod renderer;
mod world;

use wasm_bindgen::prelude::*;
use world::{Cell, World};

#[wasm_bindgen]
pub struct Simulation {
    world: World,
    frame: Vec<u8>,
}

#[wasm_bindgen]
impl Simulation {
    #[wasm_bindgen(constructor)]
    pub fn new(width: usize, height: usize) -> Simulation {
        Simulation {
            world: World::new(width, height),
            frame: vec![0; width * height * 4],
        }
    }

    /// Advance one tick and re-render into the frame buffer.
    pub fn tick(&mut self) {
        self.world.step();
        self.redraw();
    }

    /// Re-render without stepping. Used while the UI is paused.
    pub fn redraw(&mut self) {
        renderer::draw(&self.world, &mut self.frame);
    }

    /// Pointer to the RGBA frame buffer in WASM linear memory.
    /// JS must re-create its Uint8ClampedArray view each frame,
    /// because `memory.buffer` is invalidated if memory grows.
    pub fn frame_ptr(&self) -> *const u8 {
        self.frame.as_ptr()
    }

    pub fn width(&self) -> usize {
        self.world.width
    }

    pub fn height(&self) -> usize {
        self.world.height
    }

    /// Paint `material` (0=erase, 1=sand, 2=water, 3=stone, 4=fire, 5=wood,
    /// 6=steam, 7=fan→, 8=fan←, 9=fan↑, 10=fan↓, 11=gunpowder, 12=smoke,
    /// 13=oil, 14=ice, 15=glass) at (x, y).
    pub fn paint(&mut self, x: usize, y: usize, material: u8, radius: usize) {
        if let Some(cell) = Cell::from_u8(material) {
            self.world.paint(x, y, cell, radius);
        }
    }

    /// Stamp `material` along a line so fast pointer strokes do not skip cells.
    pub fn paint_line(
        &mut self,
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
        material: u8,
        radius: usize,
    ) {
        if let Some(cell) = Cell::from_u8(material) {
            self.world.paint_line(x0, y0, x1, y1, cell, radius);
        }
    }

    /// Fill a rectangle of `material`. Used by demo scenes; not on the hot path.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, material: u8) {
        if let Some(cell) = Cell::from_u8(material) {
            self.world.fill_rect(x, y, w, h, cell);
        }
    }

    /// Material id at (x, y), or 0 (empty) if out of bounds.
    pub fn cell_at(&self, x: usize, y: usize) -> u8 {
        if x >= self.world.width || y >= self.world.height {
            return 0;
        }
        self.world.get(x, y) as u8
    }

    /// Heat at (x, y), or 0 if out of bounds.
    pub fn heat_at(&self, x: usize, y: usize) -> u8 {
        if x >= self.world.width || y >= self.world.height {
            return 0;
        }
        self.world.heat()[y * self.world.width + x]
    }

    pub fn clear(&mut self) {
        self.world = World::new(self.world.width, self.world.height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_bytes(sim: &Simulation) -> Vec<u8> {
        let n = sim.width() * sim.height() * 4;
        // SAFETY: frame_ptr is sim.frame.as_ptr(); sim outlives this slice.
        unsafe { std::slice::from_raw_parts(sim.frame_ptr(), n).to_vec() }
    }

    #[test]
    fn redraw_does_not_advance_the_world() {
        let mut sim = Simulation::new(4, 4);
        sim.paint(1, 0, 1, 0);
        sim.redraw();
        let paused = frame_bytes(&sim);
        sim.redraw();
        assert_eq!(paused, frame_bytes(&sim));
        sim.tick();
        assert_ne!(paused, frame_bytes(&sim));
    }

    #[test]
    fn cell_at_and_heat_at_read_painted_cells() {
        let mut sim = Simulation::new(4, 4);
        sim.paint(2, 1, 6, 0);
        assert_eq!(sim.cell_at(2, 1), 6);
        assert_eq!(sim.heat_at(2, 1), 120);
        assert_eq!(sim.cell_at(0, 0), 0);
        assert_eq!(sim.cell_at(99, 99), 0);
        assert_eq!(sim.heat_at(99, 99), 0);
    }

    #[test]
    fn paint_line_does_not_skip_cells() {
        let mut sim = Simulation::new(8, 8);
        sim.paint_line(0, 0, 7, 7, 1, 0);
        for i in 0..8 {
            assert_eq!(sim.cell_at(i, i), 1);
        }
    }

    #[test]
    fn fill_rect_round_trips_through_cell_at() {
        let mut sim = Simulation::new(8, 8);
        sim.fill_rect(1, 1, 3, 2, 1);
        assert_eq!(sim.cell_at(1, 1), 1);
        assert_eq!(sim.cell_at(3, 2), 1);
        assert_eq!(sim.cell_at(0, 1), 0);
        assert_eq!(sim.cell_at(4, 1), 0);
    }
}
