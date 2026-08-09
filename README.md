# wasm2cl - WASM to Common Lisp transpiler

The finest way to "Heap exhausted, game over."!

## Example

```bash
# Install the wasm32-wasip1 toolchain
rustup target add wasm32-wasip1
# Build a test file
cargo build --target wasm32-wasip1 --release
# Produce `wasm2cl-wasm2cl-sys/`
# Functions are spread over multiple files to
# reduce the likelyhood of running out of memory
# during compilation.
cargo run --release -- target/wasm32-wasip1/release/wasm2cl.wasm wasm2cl-wasm2cl-sys
```

```lisp
;; system `wasm2cl-wasm2cl-sys` will be freshly created in `wasm2cl-wasm2cl-sys/`
;; and system `wasm2cl` is in `runtime/`
(asdf:load-system :wasm2cl-wasm2cl-sys)
(wasm2cl-wasip1:run :wasm2cl-wasm2cl-sys "--help")
```

## TODO

Run the standard conformance tests against this, ideally in CI too.

## Ports

There are ports of a few libraries and programs.
They require `wasi-sdk` to build, tested with 33.0.

```bash
cd ports
./build.sh <path-to-wasi-sdk>
```

This produces a few wasm files that can be translated.
- `ports/sysroot/games/prboom`
- `ports/sysroot/bin/sdlquake`

Data files for prboom available in `ports/prboom-2.5.0/data/` (Doom 1 shareware WAD)
Data files for sdlquake available in `ports/sdlquake-1.0.9/id1/` (Quake 1 shareware)

## SBCL miscompilation on arm64 (updated 2026-8-9)

sdlquake triggers a miscompilation on arm64 related to specialized entry points.
This can be worked around by passing the `--notinline` flag to `wasm2cl` to
disable this SBCL functionality.
