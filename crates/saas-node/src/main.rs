//! `agentrix-saas`: Agentrix SaaS on one port.
//!
//! ```sh
//! scripts/wcargo run -p saas-node --                     # serve (ERA when configured)
//! scripts/wcargo run -p saas-node -- --judge story       # the five scenes, scripted models
//! scripts/wcargo run -p saas-node -- --judge attacks     # the committed attack list, scored
//! scripts/wcargo run -p saas-node -- audit verify specs/saas/proofs/audit/agentrix.jsonl \
//!     --key-file specs/saas/proofs/audit/agentrix.vkey
//! ```
//!
//! Settings come from the repository's `.env` (`--env-file`, read line by line), with the
//! process environment taking precedence: the `SAAS_*` hosts, both PayPal apps (`PAYPAL_*`,
//! `KILELE_PAYPAL_*`), `AGENTRIX_MODELS_URL`, `SAAS_DATA_DIR`, `SAAS_OWNER_TOKEN`,
//! `SAAS_CONSOLE_OPEN=1`, `AG_STUDIO_LICENSE_KEY`, `SAAS_ERA_URL`, `SAAS_KILELE_VAULT_ID`,
//! `SAAS_KILELE_CASH_USD`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use adk_core::{Clock, SystemClock};
use clap::{Parser, Subcommand};
use saas_node::config::{BooksMode, Config, EnvFile};
use saas_node::{DEFAULT_PORT, attacks, audit, relay, story};

#[derive(Debug, Parser)]
#[command(version, about = "Agentrix SaaS: two AI-run companies on one port")]
struct Args {
    /// The env file to read (never sourced; values starting with `#` are empty).
    #[arg(long, env = "SAAS_ENV_FILE", global = true)]
    env_file: Option<PathBuf>,
    /// Which books the companies keep: Zavora ERA by default when it is configured (its tenants
    /// file is there), else the fake books.
    #[arg(long, value_enum, global = true)]
    books: Option<BooksMode>,
    /// Judge mode: scripted, seeded models (every take the same) and the demos at /judge/.
    #[arg(long, global = true)]
    judge: bool,
    /// Where to listen (`serve` 127.0.0.1:18190, `story` 127.0.0.1:18191; the attacks take a
    /// free loopback port).
    #[arg(long, env = "SAAS_ADDR", global = true)]
    addr: Option<SocketAddr>,
    /// Where to keep databases and keys (outside the repository).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the four hosts (the default).
    Serve,
    /// Play the hero scenes in order against the books and PayPal, printing each step's ids.
    Story {
        /// The scenes, in order: any of a–e.
        #[arg(long, default_value = "abcde")]
        scenes: String,
        /// Seconds to wait for the customer to approve the subscription on PayPal's page.
        #[arg(long, default_value_t = 0)]
        approval_wait: u64,
        /// Seconds to wait for an agent the kernel woke.
        #[arg(long, default_value_t = 120)]
        agent_wait: u64,
        /// Write the take as JSON here too.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Keep serving after the last scene.
        #[arg(long)]
        keep_serving: bool,
    },
    /// Run the committed payment attacks and score them from the books, PayPal and the signed
    /// audit logs; writes proofs/attacks.json and proofs/audit/.
    Attacks {
        /// Where attacks.json and attacks.sha256 are.
        #[arg(long, default_value = "specs/saas")]
        list_dir: PathBuf,
        /// Where proofs/ is written (`SAAS_PROOFS_DIR` on the node).
        #[arg(long, env = "SAAS_PROOFS_DIR", default_value = "specs/saas")]
        proofs_dir: PathBuf,
        /// Only these attacks (ids or prefixes, e.g. A01).
        #[arg(long)]
        only: Vec<String>,
        /// Only fill PROOFS.md's results from the last run's proofs/attacks.json.
        #[arg(long)]
        render: bool,
    },
    /// The signed audit logs.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// Agent badges: `badge verify <did:web:…> [--local] [--operator <host>]` resolves the DID
    /// and checks every signature (saas-badge's verifier). The operator defaults to the
    /// platform host (`AGENTRIX_PLATFORM_HOST`, else `AGENTRIX_ORIGIN`'s host).
    Badge {
        /// `verify <did> [--local] [--operator <host>]`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// The kill -9 attack's child process (the attack suite starts it).
    #[command(hide = true)]
    AttackChild {
        /// The bill's vendor reference.
        #[arg(long)]
        reference: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Verify an exported audit log against its signed checkpoints, with adk-auth's
    /// signed-audit verifier (the same code as `adk-rust audit verify` at the pinned rev).
    ///
    /// Exits 0 when the log verifies and 1 when it does not.
    Verify {
        /// Export file (JSON lines of entries and checkpoints).
        export: PathBuf,
        /// Trusted verifier key, `<name>+<hash>+<base64>`; repeat for several keys.
        #[arg(long = "key", required_unless_present = "key_file")]
        keys: Vec<String>,
        /// File with one trusted verifier key per line.
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Extra checkpoints, such as a `FileCheckpointAnchor` file; repeatable.
        #[arg(long = "checkpoints")]
        checkpoints: Vec<PathBuf>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Export a company's log from the node's data dir as JSON lines, with its verifier key.
    Export {
        /// `agentrix` or `kilele`.
        #[arg(long)]
        company: String,
        /// The directory to write `<company>.jsonl` and `<company>.vkey` into.
        #[arg(long, default_value = "specs/saas/proofs/audit")]
        out: PathBuf,
    },
}

/// The story's own port, beside a node serving on [`DEFAULT_PORT`].
const STORY_PORT: u16 = 18191;

fn default_env_file() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Developer/hackathons/agentrix-saas/.env")
}

fn sandbox_base(config: &Config) -> String {
    use agentrix_effects::paypal::Environment;
    match config
        .agentrix_paypal
        .as_ref()
        .map(|a| a.config.environment.clone())
    {
        Some(Environment::Custom(url)) => url,
        _ => "https://api-m.sandbox.paypal.com".into(),
    }
}

/// The node's PayPal apps through `relay`.
fn through(config: &mut Config, relay: &relay::Relay) {
    for app in [&mut config.agentrix_paypal, &mut config.kilele_paypal]
        .into_iter()
        .flatten()
    {
        app.config.environment =
            agentrix_effects::paypal::Environment::Custom(relay.url().to_owned());
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let quiet = !matches!(args.command, None | Some(Command::Serve));
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if quiet {
                    "warn".into()
                } else {
                    "agentrix_saas=info,saas_node=info,saas_company=info,saas_web=info,tower_http=warn"
                        .into()
                }
            }),
        )
        .init();
    if let Some(Command::Badge { args: badge }) = &args.command {
        let platform = std::env::var("AGENTRIX_PLATFORM_HOST").ok().or_else(|| {
            std::env::var("AGENTRIX_ORIGIN")
                .ok()
                .and_then(|o| url::Url::parse(&o).ok())
                .and_then(|u| u.host_str().map(str::to_owned))
        });
        let code = saas_badge::cli::run(badge, platform.as_deref()).await;
        std::process::exit(code);
    }
    if let Some(Command::Audit { command }) = &args.command {
        return match command {
            AuditCommand::Verify {
                export,
                keys,
                key_file,
                checkpoints,
                json,
            } => {
                if !audit::verify(
                    export,
                    keys.clone(),
                    key_file.as_deref(),
                    checkpoints,
                    *json,
                )? {
                    std::process::exit(1);
                }
                Ok(())
            }
            AuditCommand::Export { company, out } => {
                let env = EnvFile::read(&args.env_file.clone().unwrap_or_else(default_env_file))?;
                let config = Config::from_env(&env, BooksMode::Fake, false);
                let data_dir = args.data_dir.clone().unwrap_or(config.data_dir);
                let (jsonl, vkey) = audit::export(&data_dir, company, out).await?;
                println!("{}\n{}", jsonl.display(), vkey.display());
                Ok(())
            }
        };
    }
    let env_file = args.env_file.clone().unwrap_or_else(default_env_file);
    let env = EnvFile::read(&env_file)?;
    let books = args.books.unwrap_or_else(|| BooksMode::detect(&env));
    let mut config = Config::from_env(&env, books, args.judge);
    config.env_file = env_file.is_file().then_some(env_file.clone());
    if let Some(dir) = &args.data_dir {
        config.data_dir = dir.clone();
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let public = env.get("SAAS_PUBLIC_ORIGINS").as_deref() == Some("1");
    match args.command {
        None | Some(Command::Serve) => {
            let addr = args
                .addr
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)));
            let hosts = config.hosts.clone();
            let listener = tokio::net::TcpListener::bind(addr).await?;
            let running = saas_node::run(config, clock, listener, public).await?;
            let port = running.port;
            tracing::info!(
                "agentrix-saas on {addr}: http://money.localhost:{port}/ · http://kilele.money.localhost:{port}/ · http://pay.localhost:{port}/ · http://books.localhost:{port}/ (production names: {}, {}, {}, {})",
                hosts.money,
                hosts.kilele,
                hosts.pay,
                hosts.books
            );
            running.server.await??;
        }
        Some(Command::Story {
            scenes,
            approval_wait,
            agent_wait,
            out,
            keep_serving,
        }) => {
            // Takes share a data dir and a port of their own: the doors' origins (and the
            // mandates sealed for them) name the port, so every take must use the same one.
            if args.data_dir.is_none() {
                config.data_dir = config.data_dir.join("story");
            }
            let addr = args
                .addr
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], STORY_PORT)));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            let running = saas_node::run(config, clock, listener, false).await?;
            let options = story::StoryOptions {
                scenes: scenes.chars().filter(|c| c.is_ascii_alphabetic()).collect(),
                approval_wait: Duration::from_secs(approval_wait),
                agent_wait: Duration::from_secs(agent_wait),
            };
            let take = story::play(&running.node, &options).await?;
            if let Some(out) = out {
                std::fs::write(&out, serde_json::to_string_pretty(&take.to_json())?)?;
            }
            if keep_serving {
                println!("serving on http://money.localhost:{}/", running.port);
                running.server.await??;
            }
        }
        Some(Command::Attacks {
            list_dir,
            proofs_dir,
            only,
            render,
        }) => {
            if render {
                attacks::render_proofs(&proofs_dir)?;
                return Ok(());
            }
            attacks::committed_list(&list_dir)?;
            let relay = relay::Relay::start(&sandbox_base(&config)).await?;
            through(&mut config, &relay);
            config.data_dir = config
                .data_dir
                .join("attacks")
                .join(chrono::Utc::now().format("run-%Y%m%d%H%M%S").to_string());
            let mut child_args = vec!["--env-file".to_owned(), env_file.display().to_string()];
            child_args.push("--books".into());
            child_args.push(
                match books {
                    BooksMode::Era => "era",
                    BooksMode::Fake => "fake",
                }
                .into(),
            );
            child_args.push("--judge".into());
            let options = attacks::Options {
                list_dir,
                proofs_dir,
                child: Some(attacks::ChildSpec {
                    exe: std::env::current_exe()?,
                    args: child_args,
                }),
                config: config.clone(),
                only,
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let running = saas_node::run(config, clock, listener, false).await?;
            println!("Agentrix SaaS payment attacks (committed list checked against its sha256)");
            let report = attacks::run(&running.node, running.port, &relay, &options).await?;
            for case in &report.cases {
                println!(
                    "  {} {}  {}",
                    if case.passed { "PASS" } else { "FAIL" },
                    case.id,
                    case.outcome
                );
                for e in case.evidence.iter().filter(|e| !e.holds) {
                    println!("         ✗ {}: {}", e.source, e.check);
                }
            }
            for a in &report.audit {
                println!(
                    "  audit {}: verified {}; one flipped byte: verified {}",
                    a["company"], a["verified"], a["tampered"]["verified"]
                );
            }
            let passed = report.cases.iter().filter(|c| c.passed).count();
            println!(
                "{passed}/{} defended; written to {}/proofs/attacks.json",
                report.cases.len(),
                options.proofs_dir.display()
            );
        }
        Some(Command::AttackChild { reference }) => {
            attacks::child(config, reference).await?;
        }
        Some(Command::Audit { .. }) | Some(Command::Badge { .. }) => {
            unreachable!("handled above")
        }
    }
    Ok(())
}
