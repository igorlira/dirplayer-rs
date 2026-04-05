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

# Build a JSON object from env vars referenced in test configs.
# Scans all TOML configs for ${VAR_NAME...} patterns and collects
# their values from the current environment.
CONFIG_DIR="tests/e2e/configs"
ENV_VARS=$(grep -hroP '\$\{\K[A-Z_]+' "$CONFIG_DIR"/*.toml 2>/dev/null | sort -u)
TEST_ENV_JSON="{"
FIRST=true
for VAR in $ENV_VARS; do
  VAL="${!VAR:-}"
  if [ -n "$VAL" ]; then
    $FIRST || TEST_ENV_JSON="$TEST_ENV_JSON,"
    # Escape for JSON
    ESCAPED=$(printf '%s' "$VAL" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()), end="")')
    TEST_ENV_JSON="$TEST_ENV_JSON \"$VAR\": $ESCAPED"
    FIRST=false
  fi
done
TEST_ENV_JSON="$TEST_ENV_JSON }"

# Copy static files and generate index.html from template
cp "$TEMPLATE_DIR/dirplayer-js-api.js" "$RUNNER_DIR/"
sed -e "s/\$WASM_JS_FILE/$JS_BASENAME/" \
    -e "s|\$TEST_ENV_JSON|$TEST_ENV_JSON|" \
    "$TEMPLATE_DIR/index.template.html" > "$RUNNER_DIR/index.html"

# Symlink assets (same-origin)
ln -sfn "$(cd "$ASSET_DIR" && pwd)" "$RUNNER_DIR/assets"

echo "Generated test runner in $RUNNER_DIR/"

# Run Playwright
echo "Running Playwright tests..."
cd ..
npx playwright test "$@"
