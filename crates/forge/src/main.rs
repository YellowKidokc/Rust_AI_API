//! `forge`: one binary, three front doors over forge-core.
//!   forge serve            local HTTP gateway (OpenAI-compatible + the HTML pages' endpoints)
//!   forge mcp              MCP server on stdio, for other AIs
//!   forge run / ask / ...  the command line

mod mcp;
mod server;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use forge_core::pipeline::{self, RunOptions};
use forge_core::{template, Config, Forge, Message, RouteRequest, Tier};

#[derive(Parser)]
#[command(
    name = "forge",
    version,
    about = "Local AI workbench: one router, one ledger, CLI + HTTP gateway + MCP server."
)]
struct Cli {
    /// Use this folder (containing config/) instead of searching for one.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the local HTTP gateway (127.0.0.1 by default).
    Serve {
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Run the MCP server on stdio.
    Mcp,
    /// Ask one question through the router.
    Ask {
        prompt: Vec<String>,
        #[arg(long)]
        route: Option<String>,
        #[arg(long)]
        system: Option<String>,
        /// Prepend the most relevant context-library snippets.
        #[arg(long)]
        context: bool,
        #[arg(long)]
        json: bool,
        /// Never use a rung above this tier: local, free, discount, standard, premium.
        #[arg(long)]
        max_tier: Option<String>,
        /// Call a model even if this exact question was answered before.
        #[arg(long)]
        no_cache: bool,
    },
    /// Show what a route would try right now and why rungs are skipped.
    Plan {
        #[arg(long)]
        route: Option<String>,
    },
    /// Run a job template over a file or an INBOX folder; results go to OUTBOX.
    Run {
        path: PathBuf,
        #[arg(long, default_value = "default")]
        job: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        route: Option<String>,
        /// Documents in flight at once (1-60). API calls are also capped by the shared limiter.
        #[arg(long, default_value_t = 4)]
        workers: usize,
        /// Run documents even if unchanged since their last run.
        #[arg(long)]
        redo: bool,
        /// Skip the confirmation for more than 10 documents.
        #[arg(long)]
        yes: bool,
        /// Print the exact prompt for each document; call nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Context library: import, add, search.
    Context {
        #[command(subcommand)]
        cmd: CtxCmd,
    },
    /// Ledger: recent calls, totals, archive.
    Ledger {
        #[command(subcommand)]
        cmd: LedgerCmd,
    },
    /// List job templates, or show one.
    Templates { name: Option<String> },
    /// Check config, keys, database, templates.
    Doctor,
    /// Write default config/ and templates/ into a folder (never overwrites).
    Init { dir: Option<PathBuf> },
}

#[derive(Subcommand)]
enum CtxCmd {
    /// Import a .md/.json file or a folder. Markdown splits by heading; --lines makes each line a snippet.
    Import {
        path: PathBuf,
        #[arg(long, default_value = "note")]
        kind: String,
        #[arg(long)]
        lines: bool,
    },
    Add {
        key: String,
        title: String,
        body: String,
        #[arg(long, default_value = "note")]
        kind: String,
    },
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        kind: Option<String>,
    },
}

#[derive(Subcommand)]
enum LedgerCmd {
    Recent {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Stats {
        #[arg(long, default_value_t = 1)]
        days: u32,
    },
    /// Mark calls and documents older than N days archived (nothing is deleted).
    Archive {
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Logs to stderr always: stdout belongs to MCP in `forge mcp`.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FORGE_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(real_main(cli))
}

fn load_config(home: Option<&PathBuf>) -> Result<Config> {
    match home {
        Some(h) => Config::load_from(h),
        None => Config::discover(),
    }
}

async fn real_main(cli: Cli) -> Result<()> {
    if let Cmd::Init { dir } = &cli.cmd {
        return init(dir.clone().unwrap_or_else(|| PathBuf::from(".")));
    }
    let cfg = load_config(cli.home.as_ref())?;
    let forge = Forge::open(cfg)?;

    match cli.cmd {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Serve { port, host } => {
            let port = port.unwrap_or(forge.config.settings.server.port);
            server::serve(forge, &host, port).await
        }
        Cmd::Mcp => mcp::run(forge).await,
        Cmd::Ask {
            prompt,
            route,
            system,
            context,
            json,
            max_tier,
            no_cache,
        } => {
            let prompt = prompt.join(" ");
            if prompt.trim().is_empty() {
                bail!("nothing to ask");
            }
            let mut messages = Vec::new();
            if let Some(s) = system {
                messages.push(Message::system(s));
            }
            let mut user = prompt.clone();
            if context {
                let hits = forge.store.context_search(
                    &prompt,
                    forge.config.settings.context.top_k,
                    None,
                )?;
                if !hits.is_empty() {
                    eprintln!(
                        "context: {}",
                        hits.iter()
                            .map(|h| h.key.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    user = format!(
                        "Background:\n{}\n{prompt}",
                        pipeline::render_context(&hits, forge.config.settings.context.max_chars)
                    );
                }
            }
            messages.push(Message::user(user));
            let max_tier = max_tier
                .map(|t| serde_json::from_value::<Tier>(serde_json::json!(t)))
                .transpose()
                .map_err(|_| anyhow::anyhow!("unknown tier"))?;
            match forge
                .router
                .complete(RouteRequest {
                    route,
                    messages,
                    json_mode: json,
                    max_tier,
                    no_cache,
                    task: "cli:ask".into(),
                    ..Default::default()
                })
                .await
            {
                Ok(r) => {
                    println!("{}", r.reply.text);
                    eprintln!(
                        "\n— {}{} / {} ({}) · {} in / {} out · ${:.5} · call {}",
                        if r.cached { "cached · " } else { "" },
                        r.provider,
                        r.model,
                        r.tier.as_str(),
                        r.reply.prompt_tokens,
                        r.reply.completion_tokens,
                        r.cost_usd,
                        r.call_id.unwrap_or(0)
                    );
                    Ok(())
                }
                Err(f) => {
                    for a in &f.attempts {
                        eprintln!(
                            "  {} {} #{}: {}",
                            a.provider,
                            a.model,
                            a.attempt,
                            a.error.as_deref().unwrap_or("ok")
                        );
                    }
                    bail!(f.message)
                }
            }
        }
        Cmd::Plan { route } => {
            let (name, steps) = forge
                .router
                .plan(&RouteRequest {
                    route,
                    ..Default::default()
                })
                .map_err(anyhow::Error::msg)?;
            println!(
                "route: {name}   (spend today ${:.4}, budget {:?})",
                forge.store.spend_today()?,
                forge.config.settings.budget.daily_usd
            );
            for s in steps {
                println!(
                    "  {} {:<11} {:<34} {:<9} {}",
                    if s.eligible { "✔" } else { "✘" },
                    s.provider,
                    s.model,
                    s.tier.as_str(),
                    s.reason
                );
            }
            Ok(())
        }
        Cmd::Run {
            path,
            job,
            out,
            route,
            workers,
            redo,
            yes,
            dry_run,
        } => run(forge, path, job, out, route, workers, redo, yes, dry_run).await,
        Cmd::Context { cmd } => match cmd {
            CtxCmd::Import { path, kind, lines } => {
                let n = pipeline::import_context(&forge, &path, &kind, lines)?;
                println!(
                    "imported {n} snippet(s); library now holds {}",
                    forge.store.context_count()?
                );
                Ok(())
            }
            CtxCmd::Add {
                key,
                title,
                body,
                kind,
            } => {
                forge
                    .store
                    .context_put(&key, &kind, &title, &body, None, None)?;
                println!("saved {key}");
                Ok(())
            }
            CtxCmd::Search { query, limit, kind } => {
                for h in forge
                    .store
                    .context_search(&query.join(" "), limit, kind.as_deref())?
                {
                    let body: String = h.body.chars().take(160).collect();
                    println!(
                        "{:>6.2}  [{}] {} — {}\n        {}",
                        h.score,
                        h.kind,
                        h.key,
                        h.title,
                        body.replace('\n', " ")
                    );
                }
                Ok(())
            }
        },
        Cmd::Ledger { cmd } => match cmd {
            LedgerCmd::Recent { limit } => {
                for c in forge.store.recent_calls(limit)? {
                    println!(
                        "{:>5} {} {:<8} {:<10} {:<28} {:>6}/{:<6} ${:<9.5} {}",
                        c["id"],
                        c["started_at"]
                            .as_str()
                            .unwrap_or("")
                            .get(..19)
                            .unwrap_or(""),
                        c["status"].as_str().unwrap_or(""),
                        c["provider"].as_str().unwrap_or(""),
                        c["model"].as_str().unwrap_or(""),
                        c["prompt_tokens"],
                        c["completion_tokens"],
                        c["cost_usd"].as_f64().unwrap_or(0.0),
                        c["error_kind"].as_str().unwrap_or("")
                    );
                }
                Ok(())
            }
            LedgerCmd::Stats { days } => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&forge.store.stats(days)?)?
                );
                println!("spend today: ${:.4}", forge.store.spend_today()?);
                Ok(())
            }
            LedgerCmd::Archive { days } => {
                let (c, d) = forge.store.archive_older_than(days)?;
                println!("archived {c} call(s) and {d} document(s) older than {days} days");
                Ok(())
            }
        },
        Cmd::Templates { name } => {
            match name {
                Some(n) => {
                    let t = template::load(&forge.config.paths.templates_dir, &n)?;
                    println!("{}", serde_json::to_string_pretty(&t)?);
                }
                None => {
                    for n in template::list(&forge.config.paths.templates_dir) {
                        println!("{n}");
                    }
                }
            }
            Ok(())
        }
        Cmd::Doctor => {
            let p = &forge.config.paths;
            println!(
                "config:     {}",
                p.config_dir
                    .as_ref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| "built-in defaults (run `forge init`)".into())
            );
            println!("database:   {}", forge.store.path);
            println!(
                "templates:  {} ({})",
                p.templates_dir.display(),
                template::list(&p.templates_dir).join(", ")
            );
            println!(
                "brief:      {}",
                if forge.brief().is_some() {
                    "present"
                } else {
                    "missing (BRIEF.md next to config/)"
                }
            );
            println!("context:    {} snippet(s)", forge.store.context_count()?);
            println!(
                "ready:      {}",
                forge.router.available_providers().join(", ")
            );
            for (n, why) in forge.router.unavailable_providers() {
                println!("unavailable {n}: {why}");
            }
            println!(
                "default route: {}",
                forge.config.settings.default_route_name()
            );
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    forge: Arc<Forge>,
    path: PathBuf,
    job: String,
    out: Option<PathBuf>,
    route: Option<String>,
    workers: usize,
    redo: bool,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    let docs = pipeline::collect(&path)?;
    if docs.is_empty() {
        bail!("no .md/.txt documents at {}", path.display());
    }
    // Template errors surface before anything is sent.
    template::load(&forge.config.paths.templates_dir, &job)?;
    let outbox = out.unwrap_or_else(|| mcp::default_outbox(&path));
    println!(
        "{} document(s), job '{job}', output -> {}",
        docs.len(),
        outbox.display()
    );

    if !dry_run && !yes {
        let (route_name, steps) = forge
            .router
            .plan(&RouteRequest {
                route: route.clone(),
                ..Default::default()
            })
            .map_err(anyhow::Error::msg)?;
        let first = steps.iter().find(|s| s.eligible);
        let chars: usize = docs
            .iter()
            .filter_map(|d| std::fs::metadata(d).ok())
            .map(|m| m.len() as usize)
            .sum();
        let tokens_in = forge_core::types::estimate_tokens(chars);
        let est = first
            .and_then(|s| forge.router.find_entry(&s.provider, Some(&s.model)))
            .map(|e| (tokens_in as f64 * e.price_in + tokens_in as f64 * 0.2 * e.price_out) / 1e6)
            .unwrap_or(0.0);
        println!(
            "route {route_name}: first rung {} · ~{tokens_in} input tokens · est. ${est:.4}",
            first
                .map(|s| format!("{} / {}", s.provider, s.model))
                .unwrap_or_else(|| "NONE ELIGIBLE".into())
        );
        if docs.len() > 10 || est > 0.5 {
            print!("Run {} document(s)? type yes: ", docs.len());
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            if line.trim() != "yes" {
                println!("nothing sent.");
                return Ok(());
            }
        }
    }

    // Ctrl+C never kills a run (dictation tools send stray ones). Type `stop` to finish in-flight work and exit.
    let stop = Arc::new(AtomicBool::new(false));
    if !dry_run {
        tokio::spawn(async {
            loop {
                if tokio::signal::ctrl_c().await.is_err() {
                    break;
                }
                eprintln!(
                    "[Ctrl+C ignored — type `stop` and Enter to stop after in-flight documents]"
                );
            }
        });
        let s2 = stop.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if line.trim().eq_ignore_ascii_case("stop") {
                    s2.store(true, Ordering::SeqCst);
                    eprintln!("[stopping: no new documents will start]");
                    break;
                }
            }
        });
    }

    let opts = RunOptions {
        job,
        outbox,
        route,
        redo,
        dry_run,
        workers: workers.clamp(1, 60),
    };
    let results = forge
        .run_many(docs, opts, stop, move |r| {
            let t = chrono::Local::now().format("%H:%M:%S");
            match r.status.as_str() {
                "DRY_RUN" => {
                    println!(
                        "===== {} · route {} =====\n{}\n",
                        r.path,
                        r.route.as_deref().unwrap_or(""),
                        r.detail.as_deref().unwrap_or("")
                    )
                }
                "DONE" => println!(
                    "{t} DONE    {} -> {} ({} / {}, ${:.5}) route {}",
                    r.doc_id,
                    r.output.as_deref().unwrap_or(""),
                    r.provider.as_deref().unwrap_or(""),
                    r.model.as_deref().unwrap_or(""),
                    r.cost_usd,
                    r.route.as_deref().unwrap_or("")
                ),
                s => println!(
                    "{t} {s:<7} {} {}",
                    r.doc_id,
                    r.detail.as_deref().unwrap_or("")
                ),
            }
        })
        .await;
    let count = |s: &str| results.iter().filter(|r| r.status == s).count();
    let cost: f64 = results.iter().map(|r| r.cost_usd).sum();
    println!(
        "done {} · skipped {} · failed {} · stopped {} · ${cost:.5}",
        count("DONE"),
        count("SKIPPED"),
        count("FAILED"),
        count("STOPPED")
    );
    if count("FAILED") > 0 {
        std::process::exit(2);
    }
    Ok(())
}

fn init(dir: PathBuf) -> Result<()> {
    let files: Vec<(PathBuf, &str)> = [
        (
            dir.join("config").join("providers.json"),
            forge_core::config::DEFAULT_PROVIDERS,
        ),
        (
            dir.join("config").join("settings.json"),
            forge_core::config::DEFAULT_SETTINGS,
        ),
    ]
    .into_iter()
    .chain(
        template::BUILTIN
            .iter()
            .map(|(n, t)| (dir.join("templates").join(format!("{n}.md")), *t)),
    )
    .collect();
    for (path, text) in files {
        if path.exists() {
            println!("kept     {}", path.display());
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, text)?;
        println!("wrote    {}", path.display());
    }
    for d in ["INBOX", "OUTBOX"] {
        std::fs::create_dir_all(dir.join(d))?;
    }
    println!("next: set DEEPSEEK_API_KEY (and any others), then `forge doctor`.");
    Ok(())
}
