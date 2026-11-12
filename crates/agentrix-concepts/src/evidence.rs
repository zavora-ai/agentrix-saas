//! "How this meets Vultr Agent Rush · Challenge 1": each requirement, ticked from this run's own
//! records (never written in advance), and the pipeline strip's steps.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::store::{self, ConceptRow, RunRow};

/// One requirement and what this run shows for it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Row {
    /// A short key.
    pub key: &'static str,
    /// The requirement.
    pub title: &'static str,
    /// What it asks for.
    pub ask: &'static str,
    /// `met`, `live` (in progress) or `wait`.
    pub state: &'static str,
    /// The evidence, in plain words.
    pub text: String,
    /// Each page's fingerprint and receipt (the control row only).
    pub pages: Vec<PageRef>,
}

/// A finished page as the evidence shows it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PageRef {
    /// 1, 2 or 3.
    pub n: i64,
    /// `sha256:…`.
    pub digest: String,
    /// The first 12 hex digits.
    pub short: String,
    /// Where its receipt opens, when it has one.
    pub receipt_href: Option<String>,
}

/// The limits the evidence quotes.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Seconds before a designer's first build is stopped.
    pub build_secs: u64,
    /// Seconds before a fix or change is stopped.
    pub agent_secs: u64,
    /// Zavora's prompt cap.
    pub max_prompt_chars: usize,
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// The six rows for `run` (with its designs and the owner's finished changes by design).
pub fn rows(
    run: Option<&RunRow>,
    concepts: &[ConceptRow],
    changes: &[(String, i64)],
    limits: Limits,
) -> Vec<Row> {
    let started = run.is_some();
    let finished = run.is_some_and(|r| matches!(r.state.as_str(), "done" | "failed"));
    let done: Vec<&ConceptRow> = concepts.iter().filter(|c| c.status == "done").collect();
    let all_done = done.len() == 3;
    let kb = concepts.iter().map(|c| c.html_chars).sum::<i64>() / 1000;
    let writes: i64 = concepts.iter().map(|c| c.writes).sum();
    let rounds: i64 = concepts.iter().map(|c| c.rounds).sum();
    let guided: i64 = changes.iter().map(|(_, n)| n).sum();
    let mut refused: BTreeMap<String, i64> = BTreeMap::new();
    for c in concepts {
        let one: BTreeMap<String, i64> = serde_json::from_str(&c.refused).unwrap_or_default();
        for (k, v) in one {
            *refused.entry(k).or_default() += v;
        }
    }
    let state = |met: bool, live: bool| {
        if met {
            "met"
        } else if live {
            "live"
        } else {
            "wait"
        }
    };

    let designers = if concepts.is_empty() {
        "Three designers".to_owned()
    } else {
        plural(
            i64::try_from(concepts.len()).unwrap_or(0),
            "designer",
            "designers",
        )
    };
    let agent = Row {
        key: "agent",
        title: "A web-based agent does real work",
        ask: "Writing and running code",
        state: state(all_done, !concepts.is_empty()),
        text: format!(
            "{designers} wrote {kb} KB of HTML into their own files ({}). Agentrix checked each page for the basics: {} went back to the designers automatically{}.",
            plural(writes, "save", "saves"),
            plural(rounds, "fix round", "fix rounds"),
            if guided > 0 {
                format!(
                    ", and {} came from you",
                    plural(guided, "change", "changes")
                )
            } else {
                String::new()
            }
        ),
        pages: Vec::new(),
    };

    let refused_words = if refused.is_empty() {
        "none so far".to_owned()
    } else {
        refused
            .iter()
            .map(|(k, v)| format!("{k} \u{d7}{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let sandbox = Row {
        key: "sandbox",
        title: "Every action inside an isolated sandbox",
        ask: "Process isolation per job",
        state: state(all_done, !concepts.is_empty()),
        text: format!(
            "Each designer ran as its own Zavora process in its own room holding only two reference files. It could read and write only inside that room (fs_write is the one tool approved) and run no commands. Refused: {refused_words}."
        ),
        pages: Vec::new(),
    };

    let pages: Vec<PageRef> = done
        .iter()
        .filter_map(|c| {
            let digest = c.html_digest.clone()?;
            Some(PageRef {
                n: c.n,
                short: digest
                    .strip_prefix("sha256:")
                    .unwrap_or(&digest)
                    .chars()
                    .take(12)
                    .collect(),
                receipt_href: c.page_receipt.as_ref().map(|r| format!("/receipts/{r}")),
                digest,
            })
        })
        .collect();
    let planned = !concepts.is_empty();
    let control = Row {
        key: "control",
        title: "A central control layer",
        ask: "Plan, dispatch, verifiable output",
        state: state(all_done && planned, started),
        text: if planned {
            format!(
                "{} directions went to {} at once; Agentrix stores every finished page by its sha256 and signs a receipt for it.",
                concepts.len(),
                plural(
                    i64::try_from(concepts.len()).unwrap_or(0),
                    "designer",
                    "designers"
                )
            )
        } else if started {
            "The planner is reading the site's facts and choosing three directions.".to_owned()
        } else {
            "A planner chooses three directions, three designers start at once, and Agentrix stores every finished page by its sha256 and signs a receipt for it.".to_owned()
        },
        pages,
    };

    let planner_seen = run.map(|r| r.planner_seen.clone()).unwrap_or_default();
    let mut calls: Vec<String> = Vec::new();
    if !planner_seen.is_empty() {
        calls.push(format!("{planner_seen} (planner)"));
    }
    // Every designer call, recorded when it started (a call stopped by the cap still counts).
    let designer_calls: Vec<serde_json::Value> = concepts
        .iter()
        .flat_map(|c| serde_json::from_str::<Vec<serde_json::Value>>(&c.calls).unwrap_or_default())
        .collect();
    let field = |c: &serde_json::Value, k: &str| {
        c.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut designer_models: Vec<String> = designer_calls
        .iter()
        .map(|c| format!("{} / {}", field(c, "provider"), field(c, "model")))
        .collect();
    designer_models.sort();
    designer_models.dedup();
    for m in &designer_models {
        calls.push(format!("{m} (designers)"));
    }
    let n_calls =
        i64::from(!planner_seen.is_empty()) + i64::try_from(designer_calls.len()).unwrap_or(0);
    let all_vultr = (planner_seen.is_empty() || planner_seen.starts_with("vultr"))
        && designer_calls
            .iter()
            .all(|c| field(c, "provider") == "vultr");
    let vultr = Row {
        key: "vultr",
        title: "Agent model calls through Vultr Serverless Inference",
        ask: "No other model provider",
        state: state(all_vultr && all_done && n_calls > 0, n_calls > 0),
        text: if n_calls > 0 {
            format!(
                "{} so far: {}.",
                plural(n_calls, "model call", "model calls"),
                calls.join("; ")
            )
        } else {
            "No model calls yet. Each call's provider and model come from Zavora's own result and are listed here.".to_owned()
        },
        pages: Vec::new(),
    };

    let chosen = concepts.iter().find(|c| c.chosen == 1);
    let human = Row {
        key: "human",
        title: "A human approves before the final action",
        ask: "Pattern B",
        state: state(chosen.is_some(), finished),
        text: match chosen {
            Some(c) => format!(
                "You chose \u{201c}{}\u{201d}. Nothing changes on your live site yet: the Move rebuilds it with this look's colours and fonts, and you approve going live yourself, with your passkey or authenticator app.",
                c.name
            ),
            None => "No look is used until you choose one. After the Move, you approve going live yourself, with your passkey or authenticator app.".to_owned(),
        },
        pages: Vec::new(),
    };

    let key_files: i64 = concepts.iter().map(|c| c.key_checked_files).sum();
    let longest = run.map_or(0, |r| r.prompt_chars_max);
    let rooms = match run.and_then(|r| r.rooms_deleted_at.as_deref()) {
        Some(at) => format!(
            "The rooms were deleted at {} UTC.",
            at.get(11..16).unwrap_or(at)
        ),
        None if started => "The rooms are kept for your changes until you choose, then deleted (or after 24 hours).".to_owned(),
        None => "Rooms are deleted once you've chosen, or after 24 hours.".to_owned(),
    };
    let contain = Row {
        key: "contain",
        title: "Containment",
        ask: "Secrets, limits, lifecycle",
        state: state(all_done, !concepts.is_empty()),
        text: format!(
            "The model key is given only to the Zavora process, never written to a room: after every turn the room's files were read and the key was in none of them ({} checked). Each designer's first build is stopped at {} seconds and each fix or change at {}, and prompts are capped at {} characters (the longest this run: {}). Agentrix cleans every page before you see it and shows it in a sandboxed frame with no scripts. {rooms}",
            plural(key_files, "file", "files"),
            limits.build_secs,
            limits.agent_secs,
            thousands(i64::try_from(limits.max_prompt_chars).unwrap_or(i64::MAX)),
            thousands(longest),
        ),
        pages: Vec::new(),
    };
    vec![agent, sandbox, control, vultr, human, contain]
}

/// The rows for `site_id`'s newest run, read from its records.
pub async fn for_site(
    pool: &sqlx::SqlitePool,
    site_id: &str,
    limits: Limits,
) -> anyhow::Result<Vec<Row>> {
    let latest = store::latest_run(pool, site_id).await?;
    let (run, concepts) = match &latest {
        Some((run, concepts)) => (Some(run), concepts.as_slice()),
        None => (None, &[][..]),
    };
    let changes = match run {
        Some(r) => store::changes_by_concept(pool, &r.run_id).await?,
        None => Vec::new(),
    };
    Ok(rows(run, concepts, &changes, limits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_plainly() {
        assert_eq!(thousands(200_000), "200,000");
        assert_eq!(thousands(31_204), "31,204");
        assert_eq!(thousands(12), "12");
        assert_eq!(plural(1, "fix round", "fix rounds"), "1 fix round");
        assert_eq!(plural(2, "fix round", "fix rounds"), "2 fix rounds");
    }

    #[test]
    fn before_a_run_nothing_is_ticked() {
        let rows = rows(
            None,
            &[],
            &[],
            Limits {
                build_secs: 420,
                agent_secs: 300,
                max_prompt_chars: 200_000,
            },
        );
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|r| r.state == "wait"));
    }
}
