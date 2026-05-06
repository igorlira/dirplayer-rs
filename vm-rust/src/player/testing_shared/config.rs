use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
use serde::Deserialize;
use crate::player::reserve_player_mut;

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
    #[serde(default)]
    pub external_params: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MovieConfig {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestSection {
    #[serde(default)]
    pub suite: String,
    #[serde(default = "TestSection::default_timeout")]
    pub default_timeout: f64,
}

impl Default for TestSection {
    fn default() -> Self {
        TestSection {
            suite: String::new(),
            default_timeout: DEFAULT_TIMEOUT_SECS,
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
        cfg.test.suite = Self::resolve_env(&cfg.test.suite);
        cfg.external_params = cfg.external_params.into_iter()
            .map(|(k, v)| (k, Self::resolve_env(&v)))
            .collect();
        cfg.params = cfg.params.into_iter()
            .map(|(k, v)| (k, Self::resolve_env(&v)))
            .collect();
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
