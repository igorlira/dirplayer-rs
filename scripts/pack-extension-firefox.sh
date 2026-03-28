#!/bin/sh

set -e

npm run build-extension-firefox

cd dist-extension-firefox

# Remove build artifacts not needed in the extension
# Vite copies the WASM as content-script.wasm (from assetFileNames pattern)
# but the extension loads it from vm-rust/pkg/ via chrome.runtime.getURL
rm -f content-script.wasm

# Copy manifest
cp ../extension/manifest.firefox.json manifest.json

# Copy popup
mkdir -p extension
cp ../extension/index.html extension/index.html

# Copy icons
cp ../public/logo128.png .
cp ../public/logo192.png .

# Copy WASM and resources
mkdir -p vm-rust/pkg
cp ../vm-rust/pkg/vm_rust_bg.wasm vm-rust/pkg/
cp ../vm-rust/pkg/vm_rust.js vm-rust/pkg/
cp ../public/charmap-system.png .

zip -r ../dist-extension-firefox.zip . -x '*.DS_Store'
