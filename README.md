# Sandboxed

[![CI](https://github.com/chowjiaming/sandboxed/actions/workflows/ci.yml/badge.svg)](https://github.com/chowjiaming/sandboxed/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

A falling-sand cellular automaton written in Rust, compiled to
WebAssembly, and rendered in the browser.

**[Live demo](https://chowjiaming.github.io/sandboxed/)**

Paint sand, water, stone, fire, and wood onto a 320×200 grid. The
simulation runs in WASM: a flat `Vec<Cell>`, no per-frame
allocation, and a thin `wasm-bindgen` boundary so the core stays
unit-testable with plain `cargo test`.

## Why Rust → WASM

- **Performance** — dense grid, no GC pauses in the hot loop
- **Safety** — particle swaps without undefined behavior
- **Portability** — one codebase for native tests and the browser

## Controls

| Input        | Action                                      |
| ------------ | ------------------------------------------- |
| Click / drag | Paint the selected material                 |
| Sand         | Falls and piles; sinks through water        |
| Water        | Flows and spreads                           |
| Stone        | Static obstacle                             |
| Fire         | Rises, flickers, burns out                  |
| Wood         | Static; ignites from adjacent fire          |
| Erase / Clear | Remove cells                               |

## Quickstart

Requires a Rust toolchain, [`just`](https://github.com/casey/just),
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/), and
[`miniserve`](https://github.com/svenstaro/miniserve).

```bash
cargo install just wasm-pack miniserve
just serve          # http://localhost:8080
just verify         # fmt, clippy, tests, wasm-pack — same as CI
```

Pushes to `main` run that suite and deploy the site to GitHub Pages.

## Performance

Native `World::step()` at 960×600, release, 300 ticks
(`just bench`):

| Scene              | Before chunks | After chunks |
| ------------------ | ------------- | ------------ |
| Sparse (1 grain)   | 1,014 steps/s | 16,303 steps/s (~16×) |
| Dense (top quarter)| 833 steps/s   | 604 steps/s  |

Sparse worlds skip idle 16×16 chunks. A full grid still visits
every active chunk, so dense scenes are not faster. The live demo
stays at 320×200; these numbers are simulation throughput, not
browser composite FPS.

## Layout

| File              | Role                                  |
| ----------------- | ------------------------------------- |
| `src/world.rs`    | Grid storage and simulation rules     |
| `src/renderer.rs` | Grid → RGBA frame buffer              |
| `src/lib.rs`      | Thin wasm-bindgen boundary            |
| `index.html`      | Canvas, palette UI, rAF loop          |
| `justfile`        | Local tasks matching CI               |

## License

[MIT](./LICENSE) © Joseph Chow
