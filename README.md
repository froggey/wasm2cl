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
