# Sandboxed

A falling-sand cellular automaton written in Rust, compiled to
WebAssembly, rendered in the browser at 60fps. Millions of
particles, all safely sandboxed.

## Why this project exists

A fun, visual demo of Rust's strengths:

- **Performance**: flat memory, no GC pauses, near-native speed
- **Safety**: millions of particle swaps, zero segfaults
- **Portability**: one codebase → native tests + browser WASM

## Quickstart

```bash
cargo install wasm-pack miniserve
cargo test                      # run the pure-Rust unit tests
wasm-pack build --target web
miniserve .                     # or: npx serve .
```

## Controls

- Click / drag: paint the selected material
- Sand falls & piles, water flows, stone is a static obstacle
- Sand sinks through water (density rule)

## Layout

| File              | Role                                  |
| ----------------- | ------------------------------------- |
| `src/world.rs`    | Grid storage + simulation rules       |
| `src/renderer.rs` | Grid → RGBA frame buffer              |
| `src/lib.rs`      | Thin wasm-bindgen boundary            |
| `index.html`      | Canvas, palette UI, rAF loop          |
