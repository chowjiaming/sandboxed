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

    /// Paint `material` (0=erase, 1=sand, 2=water, 3=stone, 4=fire, 5=wood) at (x, y).
    pub fn paint(&mut self, x: usize, y: usize, material: u8, radius: usize) {
        if let Some(cell) = Cell::from_u8(material) {
            self.world.paint(x, y, cell, radius);
        }
    }

    pub fn clear(&mut self) {
        self.world = World::new(self.world.width, self.world.height);
    }
}
