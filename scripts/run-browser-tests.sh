#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../vm-rust"

ASSET_DIR="../public"
RUNNER_DIR="target/browser_runner"
TEMPLATE_DIR="tests/browser_templates"

# Build
echo "Building browser tests..."
cargo build --test mod --target wasm32-unknown-unknown --release 2>&1 | tail -1

# Generate JS glue
WASM_FILE=$(ls -t target/wasm32-unknown-unknown/release/deps/mod-*.wasm | head -1)
rm -rf "$RUNNER_DIR"
mkdir -p "$RUNNER_DIR"
wasm-bindgen "$WASM_FILE" --out-dir "$RUNNER_DIR" --target web

# Find the generated JS filename
JS_BASENAME=$(ls "$RUNNER_DIR"/mod-*.js | xargs -n1 basename | grep -v _bg | head -1)

# Copy static files and generate index.html from template
cp "$TEMPLATE_DIR/dirplayer-js-api.js" "$RUNNER_DIR/"
sed "s/\$WASM_JS_FILE/$JS_BASENAME/" "$TEMPLATE_DIR/index.template.html" > "$RUNNER_DIR/index.html"

# Symlink assets (same-origin)
ln -sfn "$(cd "$ASSET_DIR" && pwd)" "$RUNNER_DIR/assets"

echo "Generated test runner in $RUNNER_DIR/"

# Run Playwright
echo "Running Playwright tests..."
cd ..
npx playwright test "$@"
