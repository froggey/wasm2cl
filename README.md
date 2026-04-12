# wasm2cl - WASM to Common Lisp transpiler

The finest way to "Heap exhausted, game over."!

## Example

```bash
# Install the wasm32-wasip1 toolchain
rustup target add wasm32-wasip1
# Build a test file
cargo build --target wasm32-wasip1 --release
# Produce `out.lisp`
cargo run --release -- target/wasm32-wasip1/release/wasm2cl.wasm :wasm2cl-wasm2cl-sys
```

```lisp
;; Make sure runtime/runtime.lisp and runtime/wasip1.lisp are loaded
(load (compile-file "out.lisp"))
(wasm2cl-wasip1:run :wasm2cl-wasm2cl-sys "")
```
