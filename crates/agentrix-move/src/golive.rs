//! Charter v1 and the decision cards the Move raises (R7.1, R7.5, R5.4; ADR-012).
//!
//! The go-live card's effect names the candidate's manifest digest and charter v1's digest,
//! so its fingerprint (`EffectRequest::fingerprint`) binds the owner's approval to exactly
//! that release and those starting levels (R7.2).

use agentrix_types::{
    CardKind, Charter, DecisionCard, DecisionId, Digest, EffectRequest, JobLevel, Level, MoveId,
    QuarantineId, SiteId, canonical_json,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

/// Charter v1 with safe starting levels (R7.5, ADR-012): signing it unchanged lets the
/// WebAgents do only one thing without asking, the Guardian's undo of a bad release.
pub fn charter_v1() -> Charter {
    let job = |webagent: &str, role: &str, job: &str, level, ceiling, reason: &str| JobLevel {
        webagent: webagent.into(),
        role: role.into(),
        job: job.into(),
        level,
        ceiling,
        ceiling_reason: reason.into(),
    };
    Charter {
        jobs: vec![
            job(
                "Mover",
                "Moves and publishes your site",
                "Publish a new release",
                Level::AskFirst,
                Level::WithinBudget,
                "",
            ),
            job(
                "Editor",
                "Keeps your pages true",
                "Fix typos and out-of-date facts",
                Level::Suggest,
                Level::WithinBudget,
                "",
            ),
            job(
                "Concierge",
                "Answers your customers from your pages",
                "Hold orders from AI customers",
                Level::AskFirst,
                Level::AskFirst,
                "Orders involve money and other people, so it always asks you first.",
            ),
            job(
                "Guardian",
                "Watches your live site",
                "Undo a bad release",
                Level::UndoOnly,
                Level::UndoOnly,
                "The Guardian can only roll back to your last signed release.",
            ),
        ],
    }
}

/// Whether a level lets the job act without asking (R7.5 lists these in plain words).
pub fn acts_alone(level: Level) -> bool {
    matches!(
        level,
        Level::DoAndTell | Level::WithinBudget | Level::UndoOnly
    )
}

/// What a job does without asking, in plain words; `None` when it always asks.
pub fn alone_words(job: &JobLevel) -> Option<String> {
    match job.level {
        Level::UndoOnly => Some(format!(
            "If your live site breaks, the {} may roll it back to your last signed release, then tells you.",
            job.webagent
        )),
        Level::DoAndTell => Some(format!(
            "The {} may {} alone, then tells you.",
            job.webagent,
            job.job.to_lowercase()
        )),
        Level::WithinBudget => Some(format!(
            "The {} may {} alone, within its budget.",
            job.webagent,
            job.job.to_lowercase()
        )),
        _ => None,
    }
}

/// A charter's digest: SHA-256 of its RFC 8785 form, which the go-live fingerprint covers.
pub fn charter_digest(charter: &Charter) -> anyhow::Result<Digest> {
    Ok(Digest::of(&canonical_json(charter)?))
}

/// A site's platform address label: its domain with dots as dashes, so
/// `kilele-bakery.example` is served at `kilele-bakery-example.{platform host}` until the owner
/// points their own DNS at the platform (DNS cutover is out of scope, requirements §5).
pub fn site_alias(domain: &str) -> String {
    domain
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .replace('.', "-")
}

/// The platform's own host, from `AGENTRIX_ORIGIN` (the node's configured origin). `None`
/// for an IP or `localhost`, which can carry no wildcard address.
pub fn platform_host_from_env() -> Option<String> {
    let origin = std::env::var("AGENTRIX_ORIGIN").ok()?;
    let host = url::Url::parse(&origin)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    (host.contains('.') && host.parse::<std::net::IpAddr>().is_err()).then_some(host)
}

/// Where visitors reach a site's release: its platform address when the platform has one,
/// else its own domain.
pub fn public_address(domain: &str, platform: Option<&str>) -> String {
    match platform {
        Some(p) => format!("https://{}.{p}/", site_alias(domain)),
        None => format!("https://{}/", domain.to_ascii_lowercase()),
    }
}

/// How long a card waits for the owner.
pub const CARD_DAYS: i64 = 7;

/// How long the owner can undo a go-live (R7.4).
pub const UNDO_DAYS: i64 = 30;

/// The tool a go-live card allows.
pub const GO_LIVE_TOOL: &str = "release.go_live";

/// The tool a quarantine-restore card allows.
pub const RESTORE_TOOL: &str = "quarantine.restore";

/// What a go-live card is about.
#[derive(Debug, Clone, PartialEq)]
pub struct GoLiveFacts<'a> {
    /// The Move.
    pub move_id: &'a MoveId,
    /// The site's domain.
    pub domain: &'a str,
    /// The candidate's release number.
    pub seq: u64,
    /// The candidate's manifest digest.
    pub manifest: &'a Digest,
    /// Pages in the release.
    pub pages: usize,
    /// Site parity, 0 to 1.
    pub parity: f64,
    /// Items left behind.
    pub left_behind: usize,
    /// Their titles, most common first.
    pub left_behind_kinds: Vec<String>,
    /// The site.
    pub site: &'a SiteId,
    /// Charter v1.
    pub charter: &'a Charter,
}

/// The go-live card (R7.1): parity, what was left behind, what the WebAgents do without
/// asking, that the old site is kept, and how to undo.
pub fn go_live_card(facts: &GoLiveFacts<'_>, now: DateTime<Utc>) -> anyhow::Result<DecisionCard> {
    let charter_digest = charter_digest(facts.charter)?;
    let percent = (facts.parity * 1000.0).floor() / 10.0;
    let effect = EffectRequest {
        site: facts.site.clone(),
        webagent: "Mover".into(),
        tool: GO_LIVE_TOOL.into(),
        args: json!({
            "move": facts.move_id,
            "domain": facts.domain,
            "release": facts.manifest,
            "seq": facts.seq,
            "pages": facts.pages,
            "parity": format!("{percent}%"),
            "left_behind": facts.left_behind,
            "charter": charter_digest,
            "charter_version": 1,
            "old_site": "kept",
            "undo_days": UNDO_DAYS,
        }),
        amount: None,
        expiry: now + Duration::days(CARD_DAYS),
    };
    let alone: Vec<String> = facts.charter.jobs.iter().filter_map(alone_words).collect();
    let left = if facts.left_behind == 0 {
        "Nothing was left behind: no infection was found.".to_owned()
    } else {
        format!(
            "Left behind: {} item{} ({}). They are never copied or served.",
            facts.left_behind,
            if facts.left_behind == 1 { "" } else { "s" },
            facts.left_behind_kinds.join(", ").to_lowercase()
        )
    };
    let body = [
        format!(
            "Publish release {} of {}: {} pages at {percent}% parity with your old content.",
            facts.seq, facts.domain, facts.pages
        ),
        left,
        format!(
            "Without asking, the WebAgents will only do this: {} Everything else asks you first.",
            alone.join(" ")
        ),
        format!(
            "Visitors reach the new site at {}. Your old site is kept exactly as it is.",
            public_address(facts.domain, platform_host_from_env().as_deref())
        ),
        format!("Undo in one step for {UNDO_DAYS} days: roll back to the previous release or to your old site."),
    ]
    .join("\n");
    Ok(DecisionCard::new(
        DecisionId::generate(),
        CardKind::GoLive,
        "Go live with your new site",
        body,
        effect,
        now,
    )?)
}

/// The facts a go-live card's effect carries, read back for the LaunchGate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoLiveArgs {
    /// The Move.
    pub move_id: MoveId,
    /// The release number.
    pub seq: u64,
    /// The manifest digest.
    pub release: Digest,
    /// Charter v1's digest.
    pub charter: Digest,
}

impl GoLiveArgs {
    /// Reads a go-live card's arguments.
    pub fn of(card: &DecisionCard) -> anyhow::Result<Self> {
        anyhow::ensure!(
            card.kind == CardKind::GoLive && card.effect.tool == GO_LIVE_TOOL,
            "not a go-live card"
        );
        let args = &card.effect.args;
        let text = |key: &str| {
            args.get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("the go-live card has no {key}"))
        };
        Ok(Self {
            move_id: MoveId::new(text("move")?),
            seq: args
                .get("seq")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("the go-live card has no seq"))?,
            release: text("release")?.parse()?,
            charter: text("charter")?.parse()?,
        })
    }
}

/// A quarantine-restore card (R5.4): the owner asked to put an item back, with a reason; the
/// approval is a signed decision.
pub fn restore_card(
    site: &SiteId,
    item: &QuarantineId,
    title: &str,
    location: &str,
    reason: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<DecisionCard> {
    let effect = EffectRequest {
        site: site.clone(),
        webagent: "Mover".into(),
        tool: RESTORE_TOOL.into(),
        args: json!({ "item": item, "title": title, "where": location, "reason": reason }),
        amount: None,
        expiry: now + Duration::days(CARD_DAYS),
    };
    let body = [
        format!("Put back \"{title}\", found at {location}."),
        format!("Your reason: {reason}"),
        "It returns on the next rebuild. Your live site does not change until you approve a new release.".to_owned(),
    ]
    .join("\n");
    Ok(DecisionCard::new(
        DecisionId::generate(),
        CardKind::QuarantineRestore,
        format!("Put back: {title}"),
        body,
        effect,
        now,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_go_live_fingerprint_covers_the_release_and_the_charter() {
        let site = SiteId::new("site_1");
        let charter = charter_v1();
        let move_id = MoveId::new("move_1");
        let manifest = Digest::of(b"manifest");
        let facts = GoLiveFacts {
            move_id: &move_id,
            domain: "kilele-bakery.example",
            seq: 1,
            manifest: &manifest,
            pages: 5,
            parity: 0.973,
            left_behind: 6,
            left_behind_kinds: vec!["Spam links".into()],
            site: &site,
            charter: &charter,
        };
        let now = Utc::now();
        let card = go_live_card(&facts, now).unwrap();
        assert!(card.fingerprint_matches().unwrap());
        assert!(card.body.contains("97.3% parity"));
        assert!(card.body.contains("old site is kept"));
        assert!(card.body.contains("roll it back"));
        let args = GoLiveArgs::of(&card).unwrap();
        assert_eq!(
            (args.seq, args.release, args.charter),
            (1, manifest, charter_digest(&charter).unwrap())
        );

        let mut other = charter.clone();
        other.jobs[1].level = Level::WithinBudget;
        let changed = GoLiveFacts {
            charter: &other,
            ..facts
        };
        let card2 = go_live_card(&changed, now).unwrap();
        assert_ne!(
            card.fingerprint, card2.fingerprint,
            "a different charter is a different approval"
        );
    }
}
