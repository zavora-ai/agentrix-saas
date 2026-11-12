//! Edit before approving (04-ui-parity §3 item 7; concept/console.html's Edit).
//!
//! A raiser marks the parts of a card's proposal the owner may change ([`EditableField`]): a
//! reply's text, a collection date or time, an amount the owner may only lower. The owner
//! changes them on the card and approves the edited version. The approval signs the edited
//! effect's fingerprint with the same passkey or authenticator step as any card, the effect's
//! runner gets the edited effect, and the card and its receipt record the change as edited by
//! the owner, with the before and after ([`OwnerEdit`]). Everything else stays as raised: who it
//! goes to, the tool, the WebAgent, the expiry.

use std::collections::BTreeMap;

use agentrix_types::{Digest, EffectRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What kind of value a field holds, and so how the owner may change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditKind {
    /// Words: a reply, a message. Never empty; at most [`MAX_TEXT`] characters.
    Text,
    /// A date, `YYYY-MM-DD`.
    Date,
    /// A time of day, `HH:MM`.
    Time,
    /// The card's amount (`effect.amount`), which the owner may only lower. Typed in whole
    /// units ("4000" for KSh 4,000).
    AmountDown,
}

/// The longest text an owner's edit may hold.
pub const MAX_TEXT: usize = 2000;

/// One part of a card's proposal the owner may change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableField {
    /// Where it lives: an argument of `effect.args` (`reply`, `collect_on`), or `amount` for
    /// [`EditKind::AmountDown`].
    pub key: String,
    /// The owner's word for it: "Reply", "Collection day".
    pub label: String,
    /// What it holds.
    pub kind: EditKind,
}

impl EditableField {
    /// A field.
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: EditKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
        }
    }
}

/// One change the owner made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// The field's key.
    pub key: String,
    /// The owner's word for it.
    pub label: String,
    /// As the WebAgent proposed it, in the owner's words ("KSh 4,500").
    pub before: String,
    /// As the owner changed it.
    pub after: String,
}

/// The owner's edit of a card, recorded on the card and in its receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerEdit {
    /// What changed, field by field.
    pub changes: Vec<Change>,
    /// The fingerprint of the effect as raised.
    pub before: Digest,
    /// The fingerprint of the edited effect: what the approval signs and what runs.
    pub after: Digest,
    /// The edited effect.
    pub effect: EffectRequest,
}

/// Money as people read it: "KSh 4,500", "USD 19.77".
pub fn money_words(currency: &str, minor: i64) -> String {
    let symbol = if currency == "KES" { "KSh" } else { currency };
    let whole = (minor / 100).unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if minor < 0 { "-" } else { "" };
    let cents = (minor % 100).abs();
    if cents == 0 {
        format!("{symbol} {sign}{grouped}")
    } else {
        format!("{symbol} {sign}{grouped}.{cents:02}")
    }
}

/// The field's value in `effect`, as the edit form shows it: the text, the date, the time, or
/// the amount in whole units ("4500").
pub fn current(effect: &EffectRequest, field: &EditableField) -> String {
    match field.kind {
        EditKind::AmountDown => effect
            .amount
            .as_ref()
            .map(|m| {
                if m.minor % 100 == 0 {
                    (m.minor / 100).to_string()
                } else {
                    format!("{}.{:02}", m.minor / 100, (m.minor % 100).abs())
                }
            })
            .unwrap_or_default(),
        _ => match effect.args.get(&field.key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        },
    }
}

/// The field's value in the owner's words ("KSh 4,500" for an amount).
pub fn words(effect: &EffectRequest, field: &EditableField) -> String {
    match (field.kind, &effect.amount) {
        (EditKind::AmountDown, Some(m)) => money_words(&m.currency, m.minor),
        _ => current(effect, field),
    }
}

fn parse_minor(typed: &str) -> Option<i64> {
    let t: String = typed
        .trim()
        .trim_start_matches("KSh")
        .trim_start_matches("KES")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .collect();
    let (whole, cents) = match t.split_once('.') {
        Some((w, c)) if (1..=2).contains(&c.len()) => (w, format!("{c:0<2}")),
        Some(_) => return None,
        None => (t.as_str(), "00".to_owned()),
    };
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !cents.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(cents.parse::<i64>().ok()?)
}

/// Applies the owner's `edits` (field key to typed value) to `effect`. Fields not in
/// `editable` are refused; unchanged values are skipped. `Ok(None)` when nothing changed. The
/// error is plain words for the owner.
pub fn apply(
    effect: &EffectRequest,
    editable: &[EditableField],
    edits: &BTreeMap<String, String>,
) -> Result<Option<OwnerEdit>, String> {
    let mut edited = effect.clone();
    let mut changes = Vec::new();
    for (key, typed) in edits {
        let Some(field) = editable.iter().find(|f| &f.key == key) else {
            return Err(
                "That part of the card can't be changed. Only the boxes on the card can.".into(),
            );
        };
        let typed = typed.trim();
        if typed == current(effect, field) {
            continue;
        }
        match field.kind {
            EditKind::Text => {
                if typed.is_empty() {
                    return Err(format!("{} can't be empty.", field.label));
                }
                if typed.chars().count() > MAX_TEXT {
                    return Err(format!(
                        "{} is too long: at most {MAX_TEXT} characters.",
                        field.label
                    ));
                }
                set_arg(&mut edited, key, typed)?;
            }
            EditKind::Date => {
                if chrono::NaiveDate::parse_from_str(typed, "%Y-%m-%d").is_err() {
                    return Err(format!("{} needs a date, such as 2026-11-14.", field.label));
                }
                set_arg(&mut edited, key, typed)?;
            }
            EditKind::Time => {
                if chrono::NaiveTime::parse_from_str(typed, "%H:%M").is_err() {
                    return Err(format!("{} needs a time, such as 10:00.", field.label));
                }
                set_arg(&mut edited, key, typed)?;
            }
            EditKind::AmountDown => {
                let Some(money) = edited.amount.as_mut() else {
                    return Err("This card has no amount to change.".into());
                };
                let Some(minor) = parse_minor(typed) else {
                    return Err(format!("{} needs an amount, such as 4000.", field.label));
                };
                if minor <= 0 {
                    return Err(format!("{} must be more than 0.", field.label));
                }
                if minor > money.minor {
                    return Err(format!(
                        "You can lower {} but not raise it. It was {}.",
                        field.label.to_lowercase(),
                        money_words(&money.currency, money.minor)
                    ));
                }
                if minor == money.minor {
                    continue;
                }
                money.minor = minor;
            }
        }
        changes.push(Change {
            key: key.clone(),
            label: field.label.clone(),
            before: words(effect, field),
            after: words(&edited, field),
        });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let before = effect.fingerprint().map_err(|_| unreadable())?;
    let after = edited.fingerprint().map_err(|_| unreadable())?;
    Ok(Some(OwnerEdit {
        changes,
        before,
        after,
        effect: edited,
    }))
}

fn unreadable() -> String {
    "This card couldn't be read, so it can't be edited. Nothing was changed.".into()
}

fn set_arg(effect: &mut EffectRequest, key: &str, value: &str) -> Result<(), String> {
    match effect.args.as_object_mut() {
        Some(args) => {
            args.insert(key.to_owned(), Value::String(value.to_owned()));
            Ok(())
        }
        None => Err("This card's proposal can't be edited. Nothing was changed.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::SiteId;
    use agentrix_types::effect::Money;
    use chrono::Utc;

    fn effect() -> EffectRequest {
        EffectRequest {
            site: SiteId::new("s1"),
            webagent: "cashier".into(),
            tool: "order.confirm".into(),
            args: serde_json::json!({ "order": "o-118", "message": "Hi Amani", "collect_on": "2026-11-14" }),
            amount: Some(Money {
                currency: "KES".into(),
                minor: 450_000,
            }),
            expiry: Utc::now(),
        }
    }

    fn fields() -> Vec<EditableField> {
        vec![
            EditableField::new("message", "Message", EditKind::Text),
            EditableField::new("collect_on", "Collection day", EditKind::Date),
            EditableField::new("amount", "Amount", EditKind::AmountDown),
        ]
    }

    #[test]
    fn edits_change_only_editable_fields_and_lower_amounts() {
        let e = effect();
        let edits = BTreeMap::from([
            (
                "message".to_owned(),
                "Hi Amani, see you Saturday".to_owned(),
            ),
            ("amount".to_owned(), "4,000".to_owned()),
            ("collect_on".to_owned(), "2026-11-14".to_owned()),
        ]);
        let edit = apply(&e, &fields(), &edits).unwrap().unwrap();
        assert_eq!(edit.changes.len(), 2, "an unchanged date isn't a change");
        assert_eq!(edit.changes[0].before, "KSh 4,500");
        assert_eq!(edit.changes[0].after, "KSh 4,000");
        assert_eq!(edit.effect.amount.as_ref().unwrap().minor, 400_000);
        assert_ne!(edit.before, edit.after);
        assert_eq!(edit.effect.args["order"], "o-118");

        let up = BTreeMap::from([("amount".to_owned(), "5000".to_owned())]);
        assert!(apply(&e, &fields(), &up).unwrap_err().contains("lower"));
        let other = BTreeMap::from([("order".to_owned(), "o-999".to_owned())]);
        assert!(apply(&e, &fields(), &other).is_err());
        let bad = BTreeMap::from([("collect_on".to_owned(), "Saturday".to_owned())]);
        assert!(apply(&e, &fields(), &bad).is_err());
        let same = BTreeMap::from([("message".to_owned(), " Hi Amani ".to_owned())]);
        assert_eq!(apply(&e, &fields(), &same).unwrap(), None);
    }

    #[test]
    fn money_reads_as_people_write_it() {
        assert_eq!(money_words("KES", 450_000), "KSh 4,500");
        assert_eq!(money_words("USD", 1_977), "USD 19.77");
        assert_eq!(parse_minor("KSh 4,000"), Some(400_000));
        assert_eq!(parse_minor("19.5"), Some(1_950));
        assert_eq!(parse_minor("abc"), None);
    }
}
