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
