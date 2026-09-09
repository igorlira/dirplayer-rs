use std::collections::HashMap;
use indexmap::IndexMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use serde::Deserialize;
use crate::player::reserve_player_mut;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

const DEFAULT_TIMEOUT_SECS: f64 = 30.0;

#[cfg(not(target_arch = "wasm32"))]
static DOTENV_VARS: OnceLock<HashMap<String, String>> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn dotenv_vars() -> &'static HashMap<String, String> {
    DOTENV_VARS.get_or_init(|| {
        let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".env");
        let Ok(iter) = dotenvy::from_path_iter(&env_path) else {
            return HashMap::new();
        };
        iter.filter_map(Result::ok).collect()
    })
}

/// Test configuration loaded from a TOML file.
///
/// Each test suite has a `.toml` config in `tests/e2e/configs/`.
///
/// Example:
/// ```toml
/// [movie]
/// path = "dcr_woodpecker/habbo.dcr"
///
/// [test]
/// suite = "habbo_v7"
///
/// [external_params]
/// connection.info.host = "localhost"
///
/// [params]
/// username = "${HABBO_USERNAME:testuser}"
/// password = "${HABBO_PASSWORD:testpass}"
/// ```
///
/// String values support `${VAR:default}` env var interpolation.
#[derive(Debug, Clone, Deserialize)]
pub struct TestConfig {
    pub movie: MovieConfig,
    #[serde(default)]
    pub test: TestSection,
    /// IndexMap so `[external_params]` reaches the player in the order the
    /// TOML declares them — Director exposes that order through the indexed
    /// `externalParamName(n)` / `externalParamValue(n)`.
    #[serde(default)]
    pub external_params: IndexMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, String>,
    /// Host-page Flash/socket wiring, applied to `window.__dirplayerFlashConfig`
    /// by `apply_flash_config`. See `FlashConfig`.
    #[serde(default)]
    pub flash: FlashConfig,
}

/// The `[flash]` section: what a host page would normally hand
/// `src/services/flashPlayerManager.ts` through
/// `window.__dirplayerFlashConfig` (see `DirPlayer.configureFlash`).
///
/// The shipped manager reads that object lazily on every fetch / socket
/// resolution, so a movie whose network access only works behind a proxy
/// (CokeStudios dials its Multiuser gateway over TCP; a browser can only
/// speak WebSocket) can be wired up per-test here, instead of needing a
/// development fork of the manager with the mapping hardcoded.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlashConfig {
    /// `host:port` -> WebSocket URL, for both Ruffle's socket API and the
    /// WASM Multiuser Xtra (`window.dirplayerResolveSocketUrl`).
    #[serde(default)]
    pub socket_proxy: Vec<SocketProxyEntry>,
    /// Path-prefix fetch redirection, for assets a movie pulls from an
    /// absolute URL the test server doesn't host.
    #[serde(default)]
    pub fetch_rewrite: Vec<FetchRewriteRule>,
    /// Base of a generic CORS proxy, e.g. `http://127.0.0.1:3099/cors?url=`.
    /// Empty (the default) leaves cross-origin fetches alone.
    #[serde(default)]
    pub cors_proxy: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SocketProxyEntry {
    /// Movie-side host. `*` matches any host.
    pub host: String,
    /// Movie-side port. `0` matches any port. Written either as a TOML
    /// integer or as a string, so it can carry a `${VAR}` placeholder.
    #[serde(deserialize_with = "de_port")]
    pub port: String,
    /// WebSocket URL to dial instead. `${host}` / `${port}` expand to the
    /// movie-side values, which is how one wildcard entry can forward a
    /// whole range of ports.
    pub proxy_url: String,
}

impl SocketProxyEntry {
    /// The port as a number. An unparseable (or env-unset, hence empty) value
    /// becomes 0, which the resolver reads as the "any port" wildcard.
    pub fn port_number(&self) -> u32 {
        self.port.trim().parse().unwrap_or(0)
    }
}

/// Accept `port = 9000` and `port = "${COKESTUDIOS_GAME_PORT}"` alike.
fn de_port<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PortSpec {
        Int(u64),
        Str(String),
    }
    Ok(match PortSpec::deserialize(d)? {
        PortSpec::Int(i) => i.to_string(),
        PortSpec::Str(s) => s,
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct FetchRewriteRule {
    pub path_prefix: String,
    pub target_host: String,
    pub target_port: String,
    #[serde(default = "FetchRewriteRule::default_protocol")]
    pub target_protocol: String,
}

impl FetchRewriteRule {
    fn default_protocol() -> String { "http:".to_string() }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MovieConfig {
    pub path: String,
    /// The projector's `--do` launch argument — Lingo evaluated once, just
    /// before this movie's `prepareMovie`. Needed to reproduce a Flashpoint
    /// launch command that starts at a wrapper movie the launcher must seed
    /// (see `DirPlayer::startup_do`).
    #[serde(default)]
    pub startup_do: String,
    /// The projector's `--doBefore` — evaluated before the movie loads.
    #[serde(default)]
    pub startup_do_before: String,
    /// The projector's `--go N` — frame to jump to once the movie has started.
    /// 0 (the default) means "no jump".
    #[serde(default)]
    pub startup_go: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestSection {
    #[serde(default)]
    pub suite: String,
    #[serde(default = "TestSection::default_timeout")]
    pub default_timeout: f64,
    /// Subsystems this movie exercises, from the Type column of
    /// `docs/github_wiki/Tested-Movies.md`: `2d`, `flash`, `3d`, `havok`,
    /// `physx`, `groove3d`. A movie can carry several.
    ///
    /// `E2E_TAGS=3d,havok npm run e2e-test-browser` runs only the tests whose
    /// movie carries at least one of the listed tags, so a change to (say) the
    /// Havok port can be regression-checked without playing the entire suite --
    /// which is both slow and, at full length, prone to exhausting the browser.
    ///
    /// The runner reads this at BUILD time (`scripts/run-browser-tests.mjs`
    /// walks `tests/e2e/**` and pairs each test with the config its file
    /// includes), so filtering costs nothing at runtime.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for TestSection {
    fn default() -> Self {
        TestSection {
            suite: String::new(),
            default_timeout: DEFAULT_TIMEOUT_SECS,
            tags: Vec::new(),
        }
    }
}

impl TestSection {
    fn default_timeout() -> f64 { DEFAULT_TIMEOUT_SECS }
}

impl TestConfig {
    /// Parse a TOML string into a TestConfig, resolving `${VAR:default}`
    /// placeholders in all string values from environment variables.
    ///
    /// - `${VAR}` — replaced by the env var `VAR`; or an empty string if unset.
    /// - `${VAR:fallback}` — replaced by `VAR` if set, otherwise `fallback`.
    pub fn from_toml(toml_str: &str) -> Self {
        let mut cfg: TestConfig = toml::from_str(toml_str).expect("Failed to parse test config TOML");
        cfg.movie.path = Self::resolve_env(&cfg.movie.path);
        cfg.movie.startup_do = Self::resolve_env(&cfg.movie.startup_do);
        cfg.movie.startup_do_before = Self::resolve_env(&cfg.movie.startup_do_before);
        cfg.test.suite = Self::resolve_env(&cfg.test.suite);
        cfg.external_params = cfg.external_params.into_iter()
            .map(|(k, v)| (k, Self::resolve_env(&v)))
            .collect();
        cfg.params = cfg.params.into_iter()
            .map(|(k, v)| (k, Self::resolve_env(&v)))
            .collect();
        cfg.flash.cors_proxy = Self::resolve_env(&cfg.flash.cors_proxy);
        for entry in &mut cfg.flash.socket_proxy {
            entry.host = Self::resolve_env(&entry.host);
            entry.port = Self::resolve_env(&entry.port);
            entry.proxy_url = Self::resolve_env(&entry.proxy_url);
        }
        for rule in &mut cfg.flash.fetch_rewrite {
            rule.path_prefix = Self::resolve_env(&rule.path_prefix);
            rule.target_host = Self::resolve_env(&rule.target_host);
            rule.target_port = Self::resolve_env(&rule.target_port);
            rule.target_protocol = Self::resolve_env(&rule.target_protocol);
        }
        cfg
    }

    /// Shorthand for the snapshot suite name.
    pub fn suite(&self) -> &str {
        &self.test.suite
    }

    /// Get a param value, panicking if not found.
    pub fn param(&self, key: &str) -> &str {
        self.params.get(key)
            .unwrap_or_else(|| panic!("Missing required test param '{}'", key))
    }

    /// Apply `[external_params]` to the player, equivalent to the
    /// frontend's `set_external_params()` call.
    pub fn apply_external_params(&self) {
        if self.external_params.is_empty() {
            return;
        }
        let params = self.external_params.clone();
        reserve_player_mut(|player| {
            player.external_params = params;
        });
    }

    /// Apply `[flash]` to `window.__dirplayerFlashConfig`, the object the
    /// shipped `flashPlayerManager.ts` reads for socket-proxy mappings, fetch
    /// rewrites and the CORS proxy base — i.e. what a host page does through
    /// `DirPlayer.configureFlash`. Merges into whatever the test page already
    /// set (the browser template seeds `renderer` / `logLevel`), and only
    /// writes keys the config actually declares.
    ///
    /// Must run BEFORE `load_movie`: the movie can dial its Multiuser gateway
    /// during its own startup. No-op on native, which has no window.
    #[allow(unused_variables)]
    pub fn apply_flash_config(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;

            let f = &self.flash;
            if f.socket_proxy.is_empty() && f.fetch_rewrite.is_empty() && f.cors_proxy.is_empty() {
                return;
            }
            let Some(window) = web_sys::window() else { return };
            let key = JsValue::from_str("__dirplayerFlashConfig");
            let existing = js_sys::Reflect::get(&window, &key).unwrap_or(JsValue::UNDEFINED);
            let cfg: js_sys::Object = existing.dyn_into().unwrap_or_else(|_| js_sys::Object::new());

            let set = |obj: &js_sys::Object, k: &str, v: &JsValue| {
                let _ = js_sys::Reflect::set(obj, &JsValue::from_str(k), v);
            };

            if !f.socket_proxy.is_empty() {
                let arr = js_sys::Array::new();
                for entry in &f.socket_proxy {
                    let o = js_sys::Object::new();
                    set(&o, "host", &JsValue::from_str(&entry.host));
                    set(&o, "port", &JsValue::from_f64(entry.port_number() as f64));
                    set(&o, "proxyUrl", &JsValue::from_str(&entry.proxy_url));
                    arr.push(&o);
                }
                set(&cfg, "socketProxy", &arr);
            }
            if !f.fetch_rewrite.is_empty() {
                let arr = js_sys::Array::new();
                for rule in &f.fetch_rewrite {
                    let o = js_sys::Object::new();
                    set(&o, "pathPrefix", &JsValue::from_str(&rule.path_prefix));
                    set(&o, "targetHost", &JsValue::from_str(&rule.target_host));
                    set(&o, "targetPort", &JsValue::from_str(&rule.target_port));
                    set(&o, "targetProtocol", &JsValue::from_str(&rule.target_protocol));
                    arr.push(&o);
                }
                set(&cfg, "fetchRewriteRules", &arr);
            }
            if !f.cors_proxy.is_empty() {
                set(&cfg, "corsProxy", &JsValue::from_str(&f.cors_proxy));
            }
            let _ = js_sys::Reflect::set(&window, &key, &cfg);
        }
    }

    /// Apply `[movie] startup_do` to the player, equivalent to the frontend's
    /// `set_startup_do()` call. Must run BEFORE `load_movie`, since the payload
    /// is consumed by the movie-init sequence.
    pub fn apply_startup_do(&self) {
        let code = self.movie.startup_do.clone();
        let before = self.movie.startup_do_before.clone();
        let go = self.movie.startup_go;
        if code.is_empty() && before.is_empty() && go == 0 {
            return;
        }
        reserve_player_mut(|player| {
            if !code.is_empty() {
                player.startup_do = Some(code);
            }
            if !before.is_empty() {
                player.startup_do_before = Some(before);
            }
            if go != 0 {
                player.startup_go = Some(go);
            }
        });
    }

    /// Resolve `${VAR}` and `${VAR:default}` placeholders in a string.
    /// On native, reads from `std::env::var`. On WASM, reads from
    /// `window.__testEnv` (injected by the test runner).
    fn resolve_env(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(start) = rest.find("${") {
            result.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after.find('}').expect("Unclosed ${...} in config value");
            let token = &after[..end];
            // Only SCREAMING_SNAKE names are env placeholders — the same set
            // `scripts/run-browser-tests.mjs` harvests into `window.__testEnv`.
            // Anything else is left verbatim so a config value can carry a
            // placeholder meant for a later consumer (`[[flash.socket_proxy]]`
            // `proxy_url` expands `${host}` / `${port}` in the browser).
            let var_name = token.split(':').next().unwrap_or("");
            if var_name.is_empty()
                || !var_name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                result.push_str("${");
                result.push_str(token);
                result.push('}');
                rest = &after[end + 1..];
                continue;
            }
            let resolved = if let Some(colon) = token.find(':') {
                let var = &token[..colon];
                let default = &token[colon + 1..];
                Self::get_env(var).unwrap_or_else(|| default.to_string())
            } else {
                Self::get_env(token)
                    .unwrap_or_else(|| {
                        log::warn!("Env var '{}' not set and no default provided; using empty string", token);
                        String::new()
                    })
            };
            result.push_str(&resolved);
            rest = &after[end + 1..];
        }
        result.push_str(rest);
        result
    }

    /// Platform-appropriate env var lookup.
    fn get_env(name: &str) -> Option<String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::env::var(name)
                .ok()
                .or_else(|| dotenv_vars().get(name).cloned())
        }

        #[cfg(target_arch = "wasm32")]
        {
            let window = web_sys::window()?;
            let test_env = js_sys::Reflect::get(&window, &"__testEnv".into()).ok()?;
            if test_env.is_undefined() || test_env.is_null() { return None; }
            let val = js_sys::Reflect::get(&test_env, &name.into()).ok()?;
            val.as_string()
        }
    }
}
