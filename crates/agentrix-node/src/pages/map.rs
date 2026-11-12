//! Pure mappers from rows and `agentrix-types` values to `agentrix_console::view` models.
//!
//! No I/O and no clock: callers pass `now`, so every page can be tested with fixed values.
//! Words follow the console's honesty rule: they say only what the values show.

use agentrix_console::view;
use agentrix_types::ui::{self, ParityEvent, StepEvent, TickEvent};
use agentrix_types::{ApprovalMethod, DecisionCard, Digest, KeySet, Receipt, ReceiptKind, Subject};
use chrono::{DateTime, FixedOffset, Utc};

use super::reads::{
    BriefRow, DecisionRow, DecisionState, GauntletRow, JobLevel, Level, MeterRow, MoveRow,
    MoveStage, OwnerSession, PassedRows, QuarantineRow, QuarantineStage, SiteRow, SiteStage,
};

// ---------- Words ----------

/// A time in the owner's zone: "10:04" today, otherwise "Sat 7 Nov, 10:04".
pub fn when(at: DateTime<Utc>, offset_minutes: i32, now: DateTime<Utc>) -> String {
    let zone = FixedOffset::east_opt(offset_minutes * 60)
        .unwrap_or(FixedOffset::east_opt(0).expect("UTC"));
    let (at, now) = (at.with_timezone(&zone), now.with_timezone(&zone));
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %-d %b, %H:%M").to_string()
    }
}

/// A duration: "40 s", "1 m 48 s".
pub fn took(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} m {} s", secs / 60, secs % 60)
    }
}

/// Bytes: "512 B", "48 KB", "2.1 MB".
pub fn bytes_words(n: u64) -> String {
    match n {
        0..1_024 => format!("{n} B"),
        1_024..1_048_576 => format!("{} KB", n / 1_024),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// "sha256:7c1e…a90b" shortened to "7c1e…a90b".
pub fn short(d: &Digest) -> String {
    let hex = d.hex();
    format!("{}…{}", &hex[..4], &hex[hex.len() - 4..])
}

fn clamp32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ---------- Marks ----------

/// A WebAgent's mark. Colours are the console's role colours; the glyph is a two-letter
/// monogram. The first release draws the static layers of ADR-031's mark.
pub fn mark(webagent: &str, state: view::MarkState, state_words: &str) -> view::AgentMark {
    let color = match webagent.to_ascii_lowercase().as_str() {
        "mover" => "#2f55c4",
        "guardian" => "#45597f",
        "editor" => "#577035",
        "cashier" => "#11734a",
        "concierge" | "cakes" | "orders" => "#b72d50",
        "caretaker" => "#3d5a1f",
        "sales" => "#0f6b62",
        _ => "#38356d",
    };
    let glyph: String = webagent.chars().take(2).collect();
    view::AgentMark {
        name: webagent.to_owned(),
        glyph,
        color: color.to_owned(),
        state,
        state_words: state_words.to_owned(),
    }
}

/// The Chief WebAgent's team mark: the site's monogram on its ink.
pub fn chief(site: &SiteRow) -> view::AgentMark {
    let glyph = site
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "A".into());
    view::AgentMark {
        name: "Chief WebAgent".into(),
        glyph,
        color: "#2b1d14".into(),
        state: view::MarkState::Idle,
        state_words: String::new(),
    }
}

// ---------- Chrome ----------

/// The site as the console header shows it.
pub fn site_ref(site: &SiteRow, now: DateTime<Utc>) -> view::SiteRef {
    let state_words = match (site.stage, site.live_since) {
        (SiteStage::Setup, _) => "In setup".to_owned(),
        (SiteStage::Moving, _) => "Moving".to_owned(),
        (SiteStage::Live, Some(since)) => {
            format!("Live since {}", when(since, site.utc_offset_minutes, now))
        }
        (SiteStage::Live, None) => "Live".to_owned(),
    };
    let live_url = (site.stage == SiteStage::Live).then(|| {
        crate::launch::public_address(
            &site.host,
            crate::launch::platform_host_from_env().as_deref(),
        )
    });
    view::SiteRef {
        id: site.id.to_string(),
        host: site.host.clone(),
        name: site.name.clone(),
        state_words,
        fixture: site.fixture,
        live_url,
    }
}

/// What every console page carries.
pub fn chrome(
    site: &SiteRow,
    owner: &OwnerSession,
    active: view::NavTab,
    waiting: usize,
    now: DateTime<Utc>,
) -> view::Chrome {
    view::Chrome {
        site: site_ref(site, now),
        nav: view::Nav {
            active,
            decisions_waiting: u32::try_from(waiting).unwrap_or(u32::MAX),
        },
        owner: view::OwnerRef {
            name: owner.name.clone(),
        },
    }
}

// ---------- Receipts ----------

/// The owner's words for a receipt kind.
pub fn kind_words(kind: ReceiptKind) -> &'static str {
    match kind {
        ReceiptKind::JobFetch => "Fetch job",
        ReceiptKind::JobReplay => "Replay job",
        ReceiptKind::JobRender => "Render job",
        ReceiptKind::JobBuild => "Build job",
        ReceiptKind::MoveStep => "Move step",
        ReceiptKind::ClaimVerified => "Domain proved",
        ReceiptKind::DecisionApproved => "Decision approved",
        ReceiptKind::DecisionDeclined => "Decision declined",
        ReceiptKind::ReleaseSigned => "Release signed",
        ReceiptKind::ReleaseRollback => "Rolled back",
        ReceiptKind::QuarantineRestored => "Item restored",
        ReceiptKind::ConciergeRefusal => "The Concierge refused",
        ReceiptKind::DoorOrderHeld => "Order held for you",
        ReceiptKind::GauntletRun => "Test attacks run",
        ReceiptKind::RoutingChanged => "Model routing changed",
        ReceiptKind::ConsentGiven => "Customer consent given",
        ReceiptKind::ConsentWithdrawn => "Customer consent withdrawn",
        ReceiptKind::PaymentStarted => "Payment started",
        ReceiptKind::PaymentCaptured => "Payment received",
        ReceiptKind::PaymentRefunded => "Payment refunded",
        ReceiptKind::SubscriptionChanged => "Plan changed",
        ReceiptKind::PayoutSent => "Partner paid",
        ReceiptKind::OperatorAccess => "Zavora staff accessed your site",
        ReceiptKind::PlatformHold => "Platform hold changed",
        ReceiptKind::AccountCodeSignIn => "Signed in with a code",
        ReceiptKind::AccountRecoveryHold => "Account recovery requested",
        ReceiptKind::TeamChanged => "Your team changed",
        ReceiptKind::ChannelChanged => "A chat was linked or unlinked",
        ReceiptKind::ConceptPage => "A new look was designed",
        ReceiptKind::ConceptChosen => "You chose a new look",
    }
}

/// A receipt's title: its `detail.title` when the writer gave one, else what it is about.
pub fn receipt_title(r: &Receipt) -> String {
    if let Some(title) = r.body.title() {
        return title.to_owned();
    }
    let about = match &r.body.subject {
        Subject::Assessment(id) => format!("assessment {id}"),
        Subject::Move(id) => format!("Move {id}"),
        Subject::Job(id) => format!("job {id}"),
        Subject::Decision(id) => format!("decision {id}"),
        Subject::Site(id) => format!("site {id}"),
        Subject::Account(_) => "your account".to_owned(),
    };
    format!("{}: {about}", kind_words(r.body.kind))
}

/// Whether a receipt's signature checks against the published keys.
pub fn verified(r: &Receipt, keys: &KeySet) -> bool {
    r.verify_with(keys).is_ok()
}

/// A receipt in a list. Rows whose digest can't be computed are dropped by the caller.
pub fn receipt_row(
    r: &Receipt,
    keys: &KeySet,
    offset: i32,
    now: DateTime<Utc>,
) -> Option<view::ReceiptRow> {
    let digest = r.digest().ok()?;
    Some(view::ReceiptRow {
        digest: digest.to_string(),
        digest_short: short(&digest),
        title: receipt_title(r),
        kind_words: kind_words(r.body.kind).to_owned(),
        at_words: when(r.body.ended_at, offset, now),
        verified: verified(r, keys),
    })
}

/// One receipt, opened.
pub fn receipt_detail(
    r: &Receipt,
    keys: &KeySet,
    offset: i32,
    now: DateTime<Utc>,
) -> Option<view::ReceiptDetail> {
    let digest = r.digest().ok()?;
    let ok = verified(r, keys);
    let mut facts = Vec::new();
    if let Some(p) = &r.body.policy {
        facts.push(view::Fact {
            label: "Allowed".into(),
            value: if p.allowlist.is_empty() {
                "No network".into()
            } else {
                p.allowlist.join(", ")
            },
        });
        facts.push(view::Fact {
            label: "Runtime".into(),
            value: p.runtime.clone(),
        });
    }
    if let Some(o) = &r.body.observed {
        facts.push(view::Fact {
            label: "Hosts contacted".into(),
            value: if o.hosts.is_empty() {
                "None".into()
            } else {
                o.hosts.join(", ")
            },
        });
        facts.push(view::Fact {
            label: "Requests".into(),
            value: o.requests.to_string(),
        });
        facts.push(view::Fact {
            label: "Downloaded".into(),
            value: bytes_words(o.bytes),
        });
        facts.push(view::Fact {
            label: "Refused".into(),
            value: o.refusals.len().to_string(),
        });
    }
    if let Some(d) = r.body.destroyed {
        facts.push(view::Fact {
            label: "Room destroyed".into(),
            value: if d { "Yes".into() } else { "No".into() },
        });
    }
    Some(view::ReceiptDetail {
        digest: digest.to_string(),
        title: receipt_title(r),
        kind: r.body.kind.as_str().to_owned(),
        kind_words: kind_words(r.body.kind).to_owned(),
        at_words: format!(
            "{} – {}",
            when(r.body.started_at, offset, now),
            when(r.body.ended_at, offset, now)
        ),
        issuer: r.body.issuer.to_string(),
        subject: receipt_title(r),
        facts,
        outputs: r.body.outputs.iter().map(ToString::to_string).collect(),
        verified: ok,
        verified_words: if ok {
            format!(
                "Signature checked against our published key {}",
                r.body.issuer
            )
        } else {
            format!(
                "The signature does not check against our published keys (issuer {}). Don't rely on this receipt.",
                r.body.issuer
            )
        },
        key_id: r.body.issuer.to_string(),
        json: r
            .canonical_json()
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::to_string_pretty(&v).ok())
            .unwrap_or_default(),
    })
}

// ---------- Decisions and levels ----------

fn risk(card: &DecisionCard) -> (view::Risk, String) {
    match card.risk.as_ref().map(|r| r.score) {
        Some(s) if s >= 0.67 => (view::Risk::High, "High risk".into()),
        Some(s) if s >= 0.34 => (view::Risk::Medium, "Medium risk".into()),
        Some(_) => (view::Risk::Low, "Low risk".into()),
        None => (view::Risk::Medium, "Not scored".into()),
    }
}

fn card_state(s: DecisionState) -> view::CardState {
    match s {
        DecisionState::Waiting => view::CardState::Waiting,
        DecisionState::Approved => view::CardState::Approved,
        DecisionState::Declined => view::CardState::Declined,
        DecisionState::Expired => view::CardState::Expired,
    }
}

fn outcome_words(row: &DecisionRow) -> String {
    match (row.state, row.approval.as_ref().map(|a| a.method)) {
        (DecisionState::Approved, Some(ApprovalMethod::Passkey)) => {
            "Approved with your passkey".into()
        }
        (DecisionState::Approved, Some(ApprovalMethod::StepUp)) => {
            "Approved with a code from your authenticator app".into()
        }
        (DecisionState::Approved, None) => "Approved".into(),
        (DecisionState::Declined, _) => "Declined".into(),
        (DecisionState::Expired, _) => "Expired unanswered".into(),
        (DecisionState::Waiting, _) => String::new(),
    }
}

/// A card in the Decisions list.
pub fn card_summary(row: &DecisionRow, offset: i32, now: DateTime<Utc>) -> view::CardSummary {
    let (risk, risk_words) = risk(&row.card);
    let waiting = row.state == DecisionState::Waiting;
    view::CardSummary {
        id: row.card.decision_id.to_string(),
        title: row.card.title.clone(),
        asked_by: mark(
            &row.card.effect.webagent,
            if waiting {
                view::MarkState::Waiting
            } else {
                view::MarkState::Idle
            },
            if waiting { "Waiting for you" } else { "" },
        ),
        risk,
        risk_words,
        created_words: format!("Raised {}", when(row.card.created_at, offset, now)),
        summary: row.card.body.lines().next().unwrap_or_default().to_owned(),
        outcome_words: outcome_words(row),
    }
}

/// The owner's words for a level.
pub fn level_name(l: Level) -> &'static str {
    match l {
        Level::Watch => "Watch",
        Level::Suggest => "Suggest",
        Level::AskFirst => "Ask me first",
        Level::DoAndTell => "Do it and tell me",
        Level::WithinBudget => "Handle it within budget",
        Level::UndoOnly => "Undo only",
    }
}

/// A job's dial with the positions above its ceiling locked (ADR-012 amendment 1).
pub fn job_levels(row: &JobLevel) -> view::JobLevels {
    let ladder: &[Level] = if row.level == Level::UndoOnly || row.ceiling == Level::UndoOnly {
        &[Level::Watch, Level::Suggest, Level::UndoOnly]
    } else {
        &[
            Level::Watch,
            Level::Suggest,
            Level::AskFirst,
            Level::DoAndTell,
            Level::WithinBudget,
        ]
    };
    let positions = ladder
        .iter()
        .map(|&l| {
            let locked = l > row.ceiling;
            view::Position {
                name: level_name(l).into(),
                current: l == row.level,
                locked,
                reason: if locked {
                    row.ceiling_reason.clone()
                } else {
                    String::new()
                },
            }
        })
        .collect();
    let level_words = match row.level {
        Level::Watch => "Watch: only reports what it sees".to_owned(),
        Level::Suggest => "Suggest: drafts changes for you to approve".to_owned(),
        Level::AskFirst => "Ask me first".to_owned(),
        Level::DoAndTell => "Does reversible work alone and tells you".to_owned(),
        Level::WithinBudget => "Handles reversible work alone, within its budget".to_owned(),
        Level::UndoOnly => {
            "Undo only: may roll back to the last signed release and tell you".to_owned()
        }
    };
    view::JobLevels {
        webagent: row.webagent.clone(),
        job: row.job.clone(),
        level_words,
        above_ask_first: matches!(
            row.level,
            Level::DoAndTell | Level::WithinBudget | Level::UndoOnly
        ),
        positions,
    }
}

fn arg_fields(card: &DecisionCard, offset: i32, now: DateTime<Utc>) -> Vec<view::Fact> {
    let mut fields = vec![
        view::Fact {
            label: "WebAgent".into(),
            value: card.effect.webagent.clone(),
        },
        view::Fact {
            label: "Tool".into(),
            value: card.effect.tool.clone(),
        },
    ];
    if let Some(obj) = card.effect.args.as_object() {
        for (k, v) in obj {
            fields.push(view::Fact {
                label: k.clone(),
                value: v
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string()),
            });
        }
    }
    if let Some(m) = &card.effect.amount {
        fields.push(view::Fact {
            label: "Amount".into(),
            value: format!("{} {}", m.currency, m.minor),
        });
    }
    fields.push(view::Fact {
        label: "Expires".into(),
        value: when(card.effect.expiry, offset, now),
    });
    fields
}

/// One card, opened. `levels` are shown only on the go-live card (R7.5).
#[allow(clippy::too_many_arguments)]
pub fn card(
    row: &DecisionRow,
    levels: &[JobLevel],
    options: Option<String>,
    has_passkey: bool,
    offset: i32,
    now: DateTime<Utc>,
) -> view::Card {
    let (risk, risk_words) = risk(&row.card);
    let waiting = row.state == DecisionState::Waiting;
    let id = row.card.decision_id.to_string();
    let mut effects: Vec<String> = row
        .card
        .body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    if effects.is_empty() {
        effects.push(row.card.title.clone());
    }
    let mut facts = Vec::new();
    if let Some(r) = &row.card.risk {
        facts.push(view::Fact {
            label: "Risk check".into(),
            value: format!("{} ({})", r.summary, r.model),
        });
    }
    facts.push(view::Fact {
        label: "Expires".into(),
        value: when(row.card.effect.expiry, offset, now),
    });
    view::Card {
        id: id.clone(),
        kind: match row.card.kind {
            agentrix_types::CardKind::GoLive => view::CardKind::GoLive,
            agentrix_types::CardKind::QuarantineRestore => view::CardKind::QuarantineRestore,
            agentrix_types::CardKind::OrderHeld => view::CardKind::OrderHeld,
        },
        title: row.card.title.clone(),
        asked_by: mark(
            &row.card.effect.webagent,
            if waiting {
                view::MarkState::Waiting
            } else {
                view::MarkState::Idle
            },
            if waiting { "Waiting for you" } else { "" },
        ),
        risk,
        risk_words,
        why_words: row
            .card
            .risk
            .as_ref()
            .map(|r| r.summary.clone())
            .unwrap_or_else(|| {
                "This card hasn't been risk-scored, so it's treated as medium.".into()
            }),
        effects,
        facts,
        levels: if row.card.kind == agentrix_types::CardKind::GoLive {
            levels.iter().map(job_levels).collect()
        } else {
            Vec::new()
        },
        fingerprint: row.card.fingerprint.to_string(),
        fingerprint_short: short(&row.card.fingerprint),
        fields: arg_fields(&row.card, offset, now),
        state: card_state(row.state),
        decided: match (row.state, &row.receipt) {
            (DecisionState::Waiting, _) | (_, None) => None,
            (_, Some(receipt)) => Some(view::Decided {
                outcome_words: outcome_words(row),
                method_words: match row.approval.as_ref().map(|a| a.method) {
                    Some(ApprovalMethod::Passkey) => "Your passkey".into(),
                    Some(ApprovalMethod::StepUp) => "A code from your authenticator app".into(),
                    None => "—".into(),
                },
                at_words: row
                    .approval
                    .as_ref()
                    .map(|a| when(a.at, offset, now))
                    .unwrap_or_else(|| "—".into()),
                receipt: receipt.to_string(),
            }),
        },
        passkey_options: if waiting { options } else { None },
        has_passkey,
        approve_url: format!("/decisions/{id}/approve"),
        decline_url: format!("/decisions/{id}/decline"),
    }
}

// ---------- The Move ----------

fn step_key(s: ui::StepName) -> view::StepKey {
    match s {
        ui::StepName::Assess => view::StepKey::Assess,
        ui::StepName::Claim => view::StepKey::Claim,
        ui::StepName::Replay => view::StepKey::Replay,
        ui::StepName::Clean => view::StepKey::Clean,
        ui::StepName::Rebuild => view::StepKey::Rebuild,
        ui::StepName::Prove => view::StepKey::Prove,
        ui::StepName::GoLive => view::StepKey::Golive,
    }
}

fn step_name(s: ui::StepName) -> &'static str {
    match s {
        ui::StepName::Assess => "Assess",
        ui::StepName::Claim => "Claim",
        ui::StepName::Replay => "Replay",
        ui::StepName::Clean => "Clean",
        ui::StepName::Rebuild => "Rebuild",
        ui::StepName::Prove => "Prove",
        ui::StepName::GoLive => "Go live",
    }
}

/// A step's row.
pub fn step(e: &StepEvent) -> view::Step {
    view::Step {
        key: step_key(e.step),
        name: step_name(e.step).into(),
        state: match e.state {
            ui::StepState::Pending => view::StepState::Pending,
            ui::StepState::Running => view::StepState::Running,
            ui::StepState::Waiting => view::StepState::Waiting,
            ui::StepState::Done => view::StepState::Done,
            ui::StepState::Failed => view::StepState::Failed,
        },
        summary: e.summary.clone(),
        took_words: if e.state == ui::StepState::Pending {
            String::new()
        } else {
            took(e.elapsed_ms)
        },
        receipt: e.receipt.as_ref().map(ToString::to_string),
    }
}

/// The room's receipt, ticking.
pub fn tick(t: &TickEvent) -> view::Tick {
    view::Tick {
        hosts: clamp32(t.hosts),
        requests: clamp32(t.requests),
        bytes_words: bytes_words(t.bytes),
        refusals: clamp32(t.refusals),
    }
}

fn pct(score: f64) -> u8 {
    (score.clamp(0.0, 1.0) * 100.0).round() as u8
}

fn parity_page(p: &ParityEvent) -> view::ParityPage {
    view::ParityPage {
        url: p.page.clone().unwrap_or_default(),
        state: match p.state {
            ui::ParityState::Pass => view::ParityState::Pass,
            ui::ParityState::Retry => view::ParityState::Retry,
            ui::ParityState::Flagged => view::ParityState::Flagged,
        },
        score_words: match p.state {
            ui::ParityState::Pass => format!("{}%", pct(p.score)),
            ui::ParityState::Retry => format!("Retrying ({} of 3)", p.attempt),
            ui::ParityState::Flagged => "Flagged for you".into(),
        },
        verdict: p.verdict.clone(),
    }
}

/// The Move page. `None` when no Move has started.
pub fn move_view(
    chrome: view::Chrome,
    row: Option<&MoveRow>,
    offset: i32,
    now: DateTime<Utc>,
) -> view::MoveView {
    let Some(m) = row else {
        return view::MoveView {
            chrome,
            run: view::MoveRun {
                state: view::MoveState::None,
                ..Default::default()
            },
            ..Default::default()
        };
    };
    let site = chrome.site.id.clone();
    view::MoveView {
        chrome,
        run: view::MoveRun {
            id: m.id.to_string(),
            state: match m.stage {
                MoveStage::Running => view::MoveState::Running,
                MoveStage::Waiting => view::MoveState::Waiting,
                MoveStage::Live => view::MoveState::Live,
                MoveStage::Stopped => view::MoveState::Stopped,
            },
            started_words: when(m.started_at, offset, now),
            progress_words: m
                .pages
                .as_ref()
                .map(|p| format!("{} of {} pages", p.n, p.of)),
        },
        steps: m.steps.iter().map(step).collect(),
        tick: tick(&m.tick),
        frame: m.last_frame.as_ref().map(|f| view::Frame {
            kind: match f.kind {
                ui::FrameKind::Live => view::FrameKind::Live,
                ui::FrameKind::SealedScreenshot => view::FrameKind::SealedScreenshot,
            },
            src: f.src.clone(),
            page_url: f.page_url.clone(),
        }),
        parity: m.parity_site.as_ref().map(|s| view::Parity {
            score_words: format!("{}%", pct(s.score)),
            score_pct: pct(s.score),
            pages: m.parity_pages.iter().map(parity_page).collect(),
        }),
        quarantine: view::QuarantineSummary {
            count: u32::try_from(m.quarantined.len()).unwrap_or(u32::MAX),
            items: m
                .quarantined
                .iter()
                .map(|q| view::QuarantineLine {
                    title: q.title.clone(),
                    reason: q.reason.clone(),
                    location: q.location.clone(),
                })
                .collect(),
        },
        card: m.go_live.as_ref().map(|c| view::CardLink {
            title: c.title.clone(),
            href: format!("/console/{site}/decisions/{}", c.decision_id),
            asked_by: mark("Mover", view::MarkState::Waiting, "Waiting for you"),
        }),
        spend_words: None,
        uploads: Vec::new(),
    }
}

// ---------- Today, Trust, WebAgents, quarantine ----------

/// The Today page.
#[allow(clippy::too_many_arguments)]
pub fn today(
    chrome: view::Chrome,
    site: &SiteRow,
    owner: &OwnerSession,
    brief: Option<&BriefRow>,
    waiting: &[DecisionRow],
    recent: Vec<view::ReceiptRow>,
    passed: &PassedRows,
    now: DateTime<Utc>,
) -> view::TodayView {
    let offset = site.utc_offset_minutes;
    let hour = now
        .with_timezone(
            &FixedOffset::east_opt(offset * 60).unwrap_or(FixedOffset::east_opt(0).expect("UTC")),
        )
        .format("%H")
        .to_string()
        .parse::<u32>()
        .unwrap_or(12);
    let part = if hour < 12 {
        "Good morning"
    } else if hour < 17 {
        "Good afternoon"
    } else {
        "Good evening"
    };
    view::TodayView {
        chrome,
        greeting: format!("{part}, {}", owner.name),
        chief: chief(site),
        brief: brief.map(|b| brief_view(b, offset, now)),
        first_brief_words:
            "Your first brief comes at 07:00, written from what your WebAgents actually did.".into(),
        needs_you: waiting
            .iter()
            .map(|r| view::CardLink {
                title: r.card.title.clone(),
                href: format!("/console/{}/decisions/{}", r.card.site, r.card.decision_id),
                asked_by: mark(
                    &r.card.effect.webagent,
                    view::MarkState::Waiting,
                    "Waiting for you",
                ),
            })
            .collect(),
        recent,
        passed: passed
            .recent
            .iter()
            .map(|p| view::PassedQuestion {
                question: p.question.clone(),
                when_words: when(p.at, offset, now),
                contact: p.contact.clone(),
            })
            .collect(),
        passed_total: passed.total,
        meter: None,
    }
}

/// What each purpose is for, in the owner's words.
pub fn purpose_words(purpose: &str) -> String {
    match purpose {
        "build" => "Building pages",
        "vision-parity" => "Checking pages look right",
        "screen" => "Screening for hidden instructions",
        "decide" => "Risk decisions",
        "concierge" => "Answering visitors (Ask)",
        "brief" => "Your daily brief",
        "embed" => "Search index",
        "rerank" => "Search ranking",
        "router" => "Choosing models",
        other => other,
    }
    .to_owned()
}

/// The model-spend meter (R11.3): this month against the site's monthly AI budget, in its
/// currency (`agentrix_shell::budget`). The gateway's daily limit is an internal guard and is
/// never the owner's number.
pub fn meter(row: &MeterRow) -> view::Meter {
    use agentrix_store::words::{MicroUsd, plural};
    let b = &row.budget;
    let spent = MicroUsd(row.month.micro_usd);
    let mut calls_words = format!(
        "{} this month",
        plural(row.month.calls, "AI call", "AI calls")
    );
    if row.month.replayed > 0 {
        calls_words.push_str(&format!(
            ", {} of them replayed at no cost",
            row.month.replayed
        ));
    }
    let lines = |costs: &[(String, i64)], words: &dyn Fn(&str) -> String| {
        costs
            .iter()
            .filter(|(_, c)| *c > 0)
            .map(|(k, c)| view::MeterLine {
                label: words(k),
                amount: b.show(MicroUsd(*c)),
            })
            .collect()
    };
    view::Meter {
        spent: b.show(spent),
        cap_words: b.month_words(spent),
        cap_pct: b.percent(spent),
        cap_note: b.cap_note(),
        calls_words,
        by_purpose: lines(&row.month.by_purpose, &purpose_words),
        by_provider: lines(
            &row.month.by_provider,
            &agentrix_shell::budget::provider_words,
        ),
        refused_words: (row.month.refused > 0)
            .then(|| agentrix_shell::budget::refused_words(row.month.refused)),
    }
}

/// The brief: its headline first, then each line linked to the receipt it rests on. "Written
/// from N recorded events" counts the distinct receipts its lines cite.
pub fn brief_view(b: &BriefRow, offset: i32, now: DateTime<Utc>) -> view::Brief {
    let mut cited: Vec<&Digest> = b
        .brief
        .lines
        .iter()
        .filter_map(|l| l.receipt.as_ref())
        .collect();
    cited.sort_by_key(|d| d.to_string());
    cited.dedup();
    let headline = b.brief.headline.iter().map(|h| view::BriefLine {
        text: h.clone(),
        href: None,
    });
    let lines = b.brief.lines.iter().map(|l| view::BriefLine {
        text: l.text.clone(),
        href: l.receipt.as_ref().map(|d| format!("/receipts/{d}")),
    });
    view::Brief {
        written_words: when(b.written_at, offset, now),
        events_count: u32::try_from(cited.len()).unwrap_or(u32::MAX),
        lines: headline.chain(lines).collect(),
        spend_words: b.brief.spend_words.clone(),
    }
}

/// The Trust page.
pub fn trust(
    chrome: view::Chrome,
    gauntlet: Option<&GauntletRow>,
    receipts: Vec<view::ReceiptRow>,
    keys: &KeySet,
    offset: i32,
    now: DateTime<Utc>,
) -> view::TrustView {
    view::TrustView {
        chrome,
        gauntlet: gauntlet.map(|g| {
            let held = g.attacks.iter().filter(|a| a.held).count();
            let of = g.attacks.len();
            view::Gauntlet {
                held: u32::try_from(held).unwrap_or(u32::MAX),
                of: u32::try_from(of).unwrap_or(u32::MAX),
                score_words: format!("{held} / {of}"),
                run_words: format!("at {}", when(g.run_at, offset, now)),
                receipt: g.receipt.to_string(),
                attacks: g
                    .attacks
                    .iter()
                    .map(|a| view::Attack {
                        name: a.name.clone(),
                        plain: a.plain.clone(),
                        held: a.held,
                        receipt: a.receipt.as_ref().map(Digest::to_string),
                    })
                    .collect(),
            }
        }),
        receipts,
        key_id: keys
            .keys
            .first()
            .map(|k| k.key_id.to_string())
            .unwrap_or_else(|| "none published".into()),
    }
}

/// The WebAgents page: one card per WebAgent, its jobs in charter order.
pub fn webagents(chrome: view::Chrome, levels: &[JobLevel]) -> view::WebAgentsView {
    let mut agents: Vec<view::AgentCard> = Vec::new();
    for row in levels {
        match agents.iter_mut().find(|a| a.name == row.webagent) {
            Some(a) => a.jobs.push(job_levels(row)),
            None => agents.push(view::AgentCard {
                mark: mark(&row.webagent, view::MarkState::Idle, ""),
                name: row.webagent.clone(),
                role_words: row.role.clone(),
                jobs: vec![job_levels(row)],
            }),
        }
    }
    view::WebAgentsView { chrome, agents }
}

/// The quarantine page.
pub fn quarantine(chrome: view::Chrome, rows: &[QuarantineRow]) -> view::QuarantineView {
    let site = chrome.site.id.clone();
    view::QuarantineView {
        chrome,
        items: rows
            .iter()
            .map(|q| view::QuarantineItem {
                id: q.id.to_string(),
                title: q.title.clone(),
                plain: q.plain.clone(),
                location: q.location.clone(),
                evidence: q
                    .evidence
                    .iter()
                    .map(|d| view::Evidence {
                        label: format!("Evidence file {}", short(d)),
                        excerpt: None,
                        receipt: None,
                    })
                    .collect(),
                state: match q.stage {
                    QuarantineStage::LeftBehind => view::QuarantineState::LeftBehind,
                    QuarantineStage::RestoreAsked => view::QuarantineState::RestoreAsked,
                    QuarantineStage::Restored => view::QuarantineState::Restored,
                },
                decision_href: q
                    .decision
                    .as_ref()
                    .map(|d| format!("/console/{site}/decisions/{d}")),
                restore_url: format!("/sites/{site}/quarantine/{}/restore", q.id),
            })
            .collect(),
    }
}
