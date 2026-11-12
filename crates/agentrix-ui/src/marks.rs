//! Template functions for the living mark (ADR-031), on W1's `agentrix-marks`.
//!
//! - `wa_mark(spec, size=28, state="idle", audience="owner", font="")`: the SVG for a WebAgent's
//!   MarkSpec (the spec as stored, a map or a JSON string), safe to print.
//! - `wa_team(team, size=44, state="idle", busy=[], font="")`: the Chief WebAgent's team mark.
//! - `wa_state(state, audience)`: the state a person sees (a visitor never sees asleep, paused or
//!   frozen as themselves).
//! - `wa_words(state, audience, label="", owner="", next_wake="", reason="")`: the status line,
//!   from the real state (agentrix-marks decides what each audience reads).
//!
//! `ui.mark` and `ui.team_mark` call these when the view passes `agent.spec` or `site.team`.

use std::sync::atomic::{AtomicU64, Ordering};

use agentrix_marks::{Audience, MarkSpec, MarkState, Render, TeamMark, WordsContext};
use minijinja::value::{Kwargs, Value};
use minijinja::{Environment, Error, ErrorKind};

/// Ids inside an SVG must be unique on the page, so every rendered mark gets a fresh one.
static NEXT_UID: AtomicU64 = AtomicU64::new(1);

fn bad(msg: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidOperation, msg.into())
}

fn json_of(value: &Value) -> Result<String, Error> {
    match value.as_str() {
        Some(text) => Ok(text.to_owned()),
        None => serde_json::to_string(value).map_err(|e| bad(format!("mark spec: {e}"))),
    }
}

fn state_of(key: &str) -> Result<MarkState, Error> {
    MarkState::from_key(key).ok_or_else(|| bad(format!("unknown mark state {key:?}")))
}

fn audience_of(key: &str) -> Result<Audience, Error> {
    match key {
        "owner" => Ok(Audience::Owner),
        "visitor" => Ok(Audience::Visitor),
        other => Err(bad(format!(
            "audience is \"owner\" or \"visitor\", not {other:?}"
        ))),
    }
}

fn shown(state: MarkState, audience: Audience) -> MarkState {
    match audience {
        Audience::Visitor => state.for_visitor(),
        Audience::Owner => state,
    }
}

fn wa_mark(spec: Value, kwargs: Kwargs) -> Result<Value, Error> {
    let size: f64 = kwargs.get::<Option<f64>>("size")?.unwrap_or(28.0);
    let state = state_of(kwargs.get::<Option<&str>>("state")?.unwrap_or("idle"))?;
    let audience = audience_of(kwargs.get::<Option<&str>>("audience")?.unwrap_or("owner"))?;
    let font: &str = kwargs.get::<Option<&str>>("font")?.unwrap_or("");
    kwargs.assert_all_used()?;
    let spec = MarkSpec::from_json(&json_of(&spec)?).map_err(|e| bad(format!("mark spec: {e}")))?;
    let uid = NEXT_UID.fetch_add(1, Ordering::Relaxed).to_string();
    let svg = agentrix_marks::render(
        &spec,
        &Render {
            size,
            state: shown(state, audience),
            font,
            uid: &uid,
        },
    );
    Ok(Value::from_safe_string(svg))
}

fn wa_team(team: Value, kwargs: Kwargs) -> Result<Value, Error> {
    let size: f64 = kwargs.get::<Option<f64>>("size")?.unwrap_or(44.0);
    let state = state_of(kwargs.get::<Option<&str>>("state")?.unwrap_or("idle"))?;
    let busy: Vec<String> = kwargs
        .get::<Option<Vec<String>>>("busy")?
        .unwrap_or_default();
    let font: &str = kwargs.get::<Option<&str>>("font")?.unwrap_or("");
    kwargs.assert_all_used()?;
    let team: TeamMark =
        serde_json::from_str(&json_of(&team)?).map_err(|e| bad(format!("team mark: {e}")))?;
    let busy: Vec<&str> = busy.iter().map(String::as_str).collect();
    Ok(Value::from_safe_string(agentrix_marks::render_team(
        &team, size, state, &busy, font,
    )))
}

fn wa_state(state: &str, audience: &str) -> Result<String, Error> {
    Ok(shown(state_of(state)?, audience_of(audience)?)
        .key()
        .to_owned())
}

fn wa_words(state: &str, audience: &str, kwargs: Kwargs) -> Result<String, Error> {
    let get = |name: &str| -> Result<Option<String>, Error> {
        Ok(kwargs
            .get::<Option<String>>(name)?
            .filter(|s| !s.is_empty()))
    };
    let (label, owner, next_wake, reason) = (
        get("label")?,
        get("owner")?,
        get("next_wake")?,
        get("reason")?,
    );
    kwargs.assert_all_used()?;
    let audience = audience_of(audience)?;
    let cx = WordsContext {
        label: label.as_deref(),
        owner: owner.as_deref(),
        next_wake: next_wake.as_deref(),
        reason: reason.as_deref(),
    };
    // The words come from the real state: agentrix_marks already says what a visitor may read
    // for each (a frozen desk says "You can still read the site", not the resting line). Only
    // the mark's drawing uses the state a visitor sees (wa_state, wa_mark).
    Ok(agentrix_marks::words(state_of(state)?, audience, &cx))
}

/// Adds the mark functions to an environment.
pub(crate) fn install(env: &mut Environment<'_>) {
    env.add_function("wa_mark", wa_mark);
    env.add_function("wa_team", wa_team);
    env.add_function("wa_state", wa_state);
    env.add_function("wa_words", wa_words);
}
