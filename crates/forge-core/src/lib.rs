//! forge-core: everything the front ends share. No front end talks to a provider directly.
//!
//! * [`config`]   providers.json / settings.json (the API workbench schema) + routes
//! * [`provider`] OpenAI-compatible, Anthropic, Mock wire formats
//! * [`router`]   the cascade: eligibility, retries, fallbacks, cooldowns, quotas, budget
//! * [`limiter`]  adaptive concurrency ceiling shared by every caller in the process
//! * [`store`]    SQLite ledger, document state, context library (FTS5), candidate graph
//! * [`template`] Markdown job templates with front matter (minijinja)
//! * [`pipeline`] INBOX -> OUTBOX runs, context import
//! * [`hints`]    a note's front matter picks its own route (`priority: draft`, `compute: deep`)
//! * [`canon`]    /candidate, /search, /agenda for the Canon Workbench pages

pub mod canon;
pub mod config;
pub mod hints;
pub mod limiter;
pub mod parse;
pub mod pipeline;
pub mod provider;
pub mod router;
pub mod store;
pub mod template;
pub mod types;

use std::sync::Arc;

pub use config::{Config, Tier};
pub use router::{RouteRequest, Router};
pub use store::Store;
pub use types::Message;

/// One process-wide handle: configuration, ledger, router.
pub struct Forge {
    pub config: Config,
    pub store: Store,
    pub router: Router,
}

impl Forge {
    pub fn open(config: Config) -> anyhow::Result<Arc<Self>> {
        let store = Store::open(&config.paths.db_path())?;
        Self::with_store(config, store)
    }

    pub fn with_store(config: Config, store: Store) -> anyhow::Result<Arc<Self>> {
        let router = Router::new(config.clone(), store.clone())?;
        Ok(Arc::new(Self { config, store, router }))
    }

    /// The project brief another AI should read before working: `BRIEF.md` next to `config/`,
    /// else the context entry with key `brief`.
    pub fn brief(&self) -> Option<String> {
        if let Ok(s) = std::fs::read_to_string(self.config.paths.brief_path()) {
            return Some(s);
        }
        self.store.context_get("brief").ok().flatten().map(|h| h.body)
    }
}
