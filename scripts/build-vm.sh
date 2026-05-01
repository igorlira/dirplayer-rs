#!/bin/sh
set -e

cd vm-rust
wasm-pack build --target web --release
cd ..
