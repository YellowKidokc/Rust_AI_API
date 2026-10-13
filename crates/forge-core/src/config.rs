//! Configuration: `providers.json` + `settings.json`, in the same schema the API workbench
//! (`_system\config\`) already uses, plus the routing tables forge adds on top.
//!
//! Resolution order for the config folder:
//! 1. `FORGE_HOME` (a folder that contains `config\`)
//! 2. walk up from the working directory, then from the executable's folder; at each level take
//!    `config\providers.json` (a station BACKSIDE, a project folder) or `_system\config\providers.json`
//! 3. built-in defaults compiled into the binary
//!
//! Keys never live in these files: a provider names the environment variable that holds its key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_PROVIDERS: &str = include_str!("../../../config/providers.json");
pub const DEFAULT_SETTINGS: &str = include_str!("../../../config/settings.json");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Wire {
    Openai,
    Anthropic,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDef {
    #[serde(default)]
    pub base: String,
    #[serde(default = "default_chat")]
    pub chat: String,
    /// Name of the environment variable holding the key (never the key itself).
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default = "yes")]
    pub openai_compatible: bool,
    #[serde(default)]
    pub no_key: bool,
    #[serde(default)]
    pub alias_of: Option<String>,
    /// Explicit wire format. When absent: `openai` if `openai_compatible`, else `anthropic`
    /// for a provider whose base points at anthropic.com.
    #[serde(default)]
    pub wire: Option<Wire>,
    /// Extra headers (e.g. OpenRouter's `HTTP-Referer` / `X-Title`).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

fn default_chat() -> String {
    "v1/chat/completions".into()
}
fn yes() -> bool {
    true
}

impl ProviderDef {
    pub fn wire(&self) -> Wire {
        if let Some(w) = self.wire {
            return w;
        }
        if self.base.starts_with("mock") {
            Wire::Mock
        } else if !self.openai_compatible && self.base.contains("anthropic.com") {
            Wire::Anthropic
        } else {
            Wire::Openai
        }
    }

    pub fn url(&self) -> String {
        format!("{}/{}", self.base.trim_end_matches('/'), self.chat.trim_start_matches('/'))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Runs on this machine (Ollama, LM Studio). No cost, no quota, may be slow.
    Local,
    /// Free quota on a hosted provider (OpenRouter `:free`, Gemini free tier).
    Free,
    /// Cheap or time-discounted.
    Discount,
    #[default]
    Standard,
    Premium,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Local => "local",
            Tier::Free => "free",
            Tier::Discount => "discount",
            Tier::Standard => "standard",
            Tier::Premium => "premium",
        }
    }
}

/// One rung on a route's ladder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tier: Tier,
    /// Requests per UTC day this rung may spend (counted from the ledger). Free tiers have one.
    #[serde(default)]
    pub daily_requests: Option<u32>,
    /// Only eligible inside this UTC window, `"HH:MM-HH:MM"` (may wrap midnight).
    #[serde(default)]
    pub window_utc: Option<String>,
    /// USD per 1M input / output tokens. 0 = free. Check your provider's current price list.
    #[serde(default)]
    pub price_in: f64,
    #[serde(default)]
    pub price_out: f64,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FallbackItem {
    Name(String),
    Pair {
        provider: String,
        #[serde(default)]
        model: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryCfg {
    #[serde(default = "d_attempts")]
    pub attempts: u32,
    #[serde(default = "d_base")]
    pub base_seconds: f64,
    #[serde(default = "d_max")]
    pub max_seconds: f64,
    /// On a 429, move to the next rung at once instead of waiting out the same one.
    #[serde(default = "yes")]
    pub cascade_on_429: bool,
}
impl Default for RetryCfg {
    fn default() -> Self {
        Self { attempts: d_attempts(), base_seconds: d_base(), max_seconds: d_max(), cascade_on_429: true }
    }
}
fn d_attempts() -> u32 {
    3
}
fn d_base() -> f64 {
    2.0
}
fn d_max() -> f64 {
    30.0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BudgetCfg {
    /// Paid rungs are skipped once today's ledger spend reaches this (USD).
    #[serde(default)]
    pub daily_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LedgerCfg {
    /// Store full prompt text in SQLite. Off by default (house rule: never log full prompts).
    /// The prompt's SHA-256 is always stored.
    #[serde(default)]
    pub store_prompts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCfg {
    #[serde(default = "d_top_k")]
    pub top_k: usize,
    #[serde(default = "d_ctx_chars")]
    pub max_chars: usize,
}
impl Default for ContextCfg {
    fn default() -> Self {
        Self { top_k: d_top_k(), max_chars: d_ctx_chars() }
    }
}
fn d_top_k() -> usize {
    5
}
fn d_ctx_chars() -> usize {
    6000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCfg {
    #[serde(default = "d_port")]
    pub port: u16,
    /// Browser origins allowed besides `null` (file:// pages) and http://127.0.0.1 / http://localhost.
    #[serde(default)]
    pub cors_origins: Vec<String>,
}
impl Default for ServerCfg {
    fn default() -> Self {
        Self { port: d_port(), cors_origins: vec![] }
    }
}
fn d_port() -> u16 {
    8790
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "d_conc")]
    pub max_concurrent_calls: usize,
    #[serde(default)]
    pub turbo_presets: BTreeMap<String, usize>,
    #[serde(default)]
    pub default_provider: Option<String>,
    /// Kept as raw JSON: the workbench's shape for this key varies between versions.
    #[serde(default)]
    pub models: serde_json::Value,
    #[serde(default)]
    pub fallback: Vec<FallbackItem>,
    #[serde(default)]
    pub allowed_providers: Vec<String>,
    #[serde(default = "d_cap")]
    pub output_token_cap: u32,
    #[serde(default)]
    pub retry: RetryCfg,
    #[serde(default = "d_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default = "d_temp")]
    pub temperature: f32,

    // ---- forge additions ----
    #[serde(default)]
    pub routes: BTreeMap<String, Vec<RouteEntry>>,
    #[serde(default)]
    pub default_route: Option<String>,
    #[serde(default)]
    pub budget: BudgetCfg,
    #[serde(default)]
    pub ledger: LedgerCfg,
    #[serde(default)]
    pub context: ContextCfg,
    #[serde(default)]
    pub server: ServerCfg,
}

fn d_conc() -> usize {
    8
}
fn d_cap() -> u32 {
    8000
}
fn d_timeout() -> u64 {
    300
}
fn d_temp() -> f32 {
    0.3
}

impl Settings {
    /// The route name used when a caller doesn't name one.
    pub fn default_route_name(&self) -> String {
        self.default_route.clone().unwrap_or_else(|| "default".into())
    }

    /// If `settings.json` has no `routes`, derive `default` from `default_provider` + `fallback[]`,
    /// so an existing workbench settings file works unchanged.
    pub fn ensure_default_route(&mut self, providers: &BTreeMap<String, ProviderDef>) {
        let name = self.default_route_name();
        if self.routes.contains_key(&name) {
            return;
        }
        let mut ladder = Vec::new();
        if let Some(p) = &self.default_provider {
            ladder.push(simple_entry(p.clone(), self.model_for(p, providers)));
        }
        for f in &self.fallback {
            let (p, m) = match f {
                FallbackItem::Name(p) => (p.clone(), None),
                FallbackItem::Pair { provider, model } => (provider.clone(), model.clone()),
            };
            let m = m.or_else(|| self.model_for(&p, providers));
            ladder.push(simple_entry(p, m));
        }
        if !ladder.is_empty() {
            self.routes.insert(name, ladder);
        }
    }

    fn model_for(&self, provider: &str, providers: &BTreeMap<String, ProviderDef>) -> Option<String> {
        if let Some(m) = self.models.get(provider).and_then(|v| v.as_str()) {
            return Some(m.to_string());
        }
        providers.get(provider).and_then(|d| d.default_model.clone())
    }
}

fn simple_entry(provider: String, model: Option<String>) -> RouteEntry {
    RouteEntry {
        provider,
        model,
        tier: Tier::Standard,
        daily_requests: None,
        window_utc: None,
        price_in: 0.0,
        price_out: 0.0,
        context_window: None,
        max_output: None,
    }
}

/// Where everything lives on disk.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Folder that holds `config/` (an `_system` folder, a station BACKSIDE, or the repo root).
    pub base: PathBuf,
    /// `None` when running on built-in defaults.
    pub config_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub templates_dir: PathBuf,
}

impl Paths {
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("forge.sqlite")
    }
    pub fn receipts_path(&self) -> PathBuf {
        self.logs_dir.join("forge_receipts.jsonl")
    }
    pub fn brief_path(&self) -> PathBuf {
        self.base.join("BRIEF.md")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub providers: BTreeMap<String, ProviderDef>,
    pub settings: Settings,
    pub paths: Paths,
}

impl Config {
    /// Find and load configuration (see module docs for the search order).
    pub fn discover() -> Result<Self> {
        let base = find_base();
        match base {
            Some(base) => Self::load_from(&base),
            None => {
                let base = std::env::current_dir()?;
                Self::from_strings(DEFAULT_PROVIDERS, DEFAULT_SETTINGS, base, None)
            }
        }
    }

    /// Load from `<base>/config/{providers,settings}.json`.
    pub fn load_from(base: &Path) -> Result<Self> {
        let cfg = base.join("config");
        let providers = std::fs::read_to_string(cfg.join("providers.json"))
            .with_context(|| format!("reading {}", cfg.join("providers.json").display()))?;
        let settings = match std::fs::read_to_string(cfg.join("settings.json")) {
            Ok(s) => s,
            Err(_) => DEFAULT_SETTINGS.to_string(),
        };
        Self::from_strings(&providers, &settings, base.to_path_buf(), Some(cfg))
    }

    pub fn from_strings(providers: &str, settings: &str, base: PathBuf, config_dir: Option<PathBuf>) -> Result<Self> {
        let providers: BTreeMap<String, ProviderDef> = serde_json::from_str(strip_bom(providers)).context("parsing providers.json")?;
        let mut settings: Settings = serde_json::from_str(strip_bom(settings)).context("parsing settings.json")?;
        settings.ensure_default_route(&providers);
        let templates_dir = [base.join("templates"), base.join("config").join("templates")]
            .into_iter()
            .find(|p| p.is_dir())
            .unwrap_or_else(|| base.join("templates"));
        let paths = Paths { data_dir: base.join("_data"), logs_dir: base.join("LOGS"), templates_dir, config_dir, base };
        Ok(Self { providers, settings, paths })
    }

    /// Resolve `alias_of` chains to the concrete definition.
    pub fn provider(&self, name: &str) -> Option<&ProviderDef> {
        let mut def = self.providers.get(name)?;
        for _ in 0..8 {
            match &def.alias_of {
                Some(target) => def = self.providers.get(target)?,
                None => return Some(def),
            }
        }
        None
    }

    pub fn provider_allowed(&self, name: &str) -> bool {
        let allowed = &self.settings.allowed_providers;
        allowed.is_empty() || allowed.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

fn find_base() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("FORGE_HOME") {
        let p = PathBuf::from(home);
        if p.join("config").join("providers.json").is_file() {
            return Some(p);
        }
    }
    // Working directory first (like git), then the executable's folder (a station's BACKSIDE).
    // At each level a `config/` beside you wins over an `_system/config/` further up.
    let mut starts = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    for start in &starts {
        for dir in start.ancestors() {
            if dir.join("config").join("providers.json").is_file() {
                return Some(dir.to_path_buf());
            }
            let sys = dir.join("_system");
            if sys.join("config").join("providers.json").is_file() {
                return Some(sys);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_defaults_parse_and_have_a_default_route() {
        let c = Config::from_strings(DEFAULT_PROVIDERS, DEFAULT_SETTINGS, PathBuf::from("."), None).unwrap();
        let name = c.settings.default_route_name();
        assert!(c.settings.routes.contains_key(&name), "missing route {name}");
        for ladder in c.settings.routes.values() {
            for e in ladder {
                assert!(c.providers.contains_key(&e.provider), "route names unknown provider {}", e.provider);
            }
        }
    }

    #[test]
    fn workbench_settings_without_routes_derive_one() {
        let providers = r#"{"deepseek":{"base":"https://api.deepseek.com","chat":"chat/completions","key":"DEEPSEEK_API_KEY","default_model":"deepseek-chat"},
                            "openai":{"base":"https://api.openai.com","key":"OPENAI_API_KEY","default_model":"gpt-x"}}"#;
        let settings = r#"{"default_provider":"deepseek","fallback":["openai"],"unknown_key":1}"#;
        let c = Config::from_strings(providers, settings, PathBuf::from("."), None).unwrap();
        let r = &c.settings.routes["default"];
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].model.as_deref(), Some("deepseek-chat"));
        assert_eq!(r[1].provider, "openai");
        assert_eq!(c.providers["deepseek"].url(), "https://api.deepseek.com/chat/completions");
    }
}
