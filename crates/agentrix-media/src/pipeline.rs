//! Making a picture: check the brief, draw, screen, describe, label. Nothing here stores or
//! publishes; [`crate::library`] keeps what was made as a draft for the owner.

use serde::Serialize;

use crate::client::{Description, DrawRequest, Drawn, Gateway, GatewayError, Verdict};
use crate::policy::{self, Kind, Refusal};

/// What someone asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// Where it goes.
    pub kind: Kind,
    /// What it should show, in the asker's words.
    pub brief: String,
    /// The site's look, added to every prompt ("warm cream and terracotta tones").
    pub look: String,
    /// What the picture is for, for the safety check and the alt text ("Product photo for
    /// Country sourdough, KSh 450"). Data, never instructions.
    pub context: String,
    /// How many to draw, 1 to 4.
    pub n: u8,
    /// A seed for a repeatable picture; one is chosen and kept when `None`.
    pub seed: Option<i64>,
    /// `fast` or `standard`.
    pub tier: &'static str,
    /// Whether the site is a sample (always labelled).
    pub sample: bool,
}

/// One picture that passed the safety check, ready to keep as a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Made {
    /// The bytes.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    /// Its type.
    pub media_type: String,
    /// Width in pixels, when known.
    pub width: Option<u32>,
    /// Height in pixels, when known.
    pub height: Option<u32>,
    /// What the vision model said.
    pub description: Description,
    /// The label a visitor sees, if any.
    pub label: Option<&'static str>,
    /// The prompt the model was sent.
    pub prompt: String,
    /// The model that drew it.
    pub model: String,
    /// The seed it was drawn with.
    pub seed: Option<i64>,
    /// The safety verdict.
    pub verdict: Verdict,
    /// Its share of the drawing cost, in micro-USD.
    pub cost_micro_usd: i64,
}

/// A picture the safety check stopped. Its bytes are not kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Blocked {
    /// The verdict.
    pub verdict: Verdict,
    /// The prompt.
    pub prompt: String,
    /// The model that drew it.
    pub model: String,
}

/// What came of one picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Passed; kept as a draft.
    Made(Box<Made>),
    /// Stopped by the safety check.
    Blocked(Blocked),
}

/// Why nothing was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MakeError {
    /// The brief asked for something the policy refuses.
    Refused(Refusal),
    /// The gateway could not draw, screen or describe.
    Gateway(GatewayError),
}

impl std::fmt::Display for MakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MakeError::Refused(r) => r.fmt(f),
            MakeError::Gateway(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MakeError {}

impl From<GatewayError> for MakeError {
    fn from(e: GatewayError) -> Self {
        MakeError::Gateway(e)
    }
}

/// A seed when the asker gave none, so every picture can be drawn again.
fn fresh_seed() -> i64 {
    (uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff) as i64
}

/// Makes `ask.n` pictures.
pub async fn make(gateway: &Gateway, ask: &Ask) -> Result<Vec<Outcome>, MakeError> {
    policy::check(ask.kind, &ask.brief).map_err(MakeError::Refused)?;
    let prompt = policy::prompt(ask.kind, &ask.brief, &ask.look);
    let seed = ask.seed.unwrap_or_else(fresh_seed);
    let request = DrawRequest {
        prompt: prompt.clone(),
        size: ask.kind.size().to_owned(),
        n: ask.n.clamp(1, 4),
        seed: Some(seed),
        tier: ask.tier,
    };
    let drawn = gateway.draw(&request).await?;
    let mut out = Vec::with_capacity(drawn.len());
    for (i, picture) in drawn.into_iter().enumerate() {
        // Pictures drawn together share the seed; record each one's place after it.
        let seed = seed + i as i64;
        let mut outcome = finish(gateway, ask, &prompt, picture, seed).await?;
        if let Outcome::Made(made) = &outcome
            && made.description.shows_person
            && ask.kind != Kind::Film
        {
            // Never real people: draw once more with another seed before asking the owner.
            let again = DrawRequest {
                n: 1,
                seed: Some(seed + 1_000),
                ..request.clone()
            };
            if let Some(retry) = gateway.draw(&again).await?.into_iter().next() {
                outcome = finish(gateway, ask, &prompt, retry, seed + 1_000).await?;
            }
        }
        out.push(outcome);
    }
    Ok(out)
}

async fn finish(
    gateway: &Gateway,
    ask: &Ask,
    prompt: &str,
    picture: Drawn,
    seed: i64,
) -> Result<Outcome, MakeError> {
    let verdict = gateway
        .screen(&picture.bytes, &picture.media_type, &ask.context)
        .await?;
    if !verdict.safe {
        return Ok(Outcome::Blocked(Blocked {
            verdict,
            prompt: prompt.to_owned(),
            model: picture.model,
        }));
    }
    // The vision model sees what was asked for, so its note flags a picture that shows
    // something the product or place doesn't have.
    let asked = format!("{} What was asked for: {}", ask.context, ask.brief.trim());
    let mut description = match gateway
        .describe(&picture.bytes, &picture.media_type, asked.trim())
        .await
    {
        Ok(description) => description,
        // A reply cut short or garbled is worth one more try before giving up.
        Err(e) if e.code == "describe_failed" => {
            gateway
                .describe(&picture.bytes, &picture.media_type, asked.trim())
                .await?
        }
        Err(e) => return Err(e.into()),
    };
    if description.shows_person && description.check.is_empty() && ask.kind != Kind::Film {
        description.check =
            "A person appears in this picture. Make sure nobody could take them for someone real."
                .to_owned();
    }
    let (width, height) = dimensions(picture.size.as_deref().unwrap_or(ask.kind.size()));
    let label = policy::label(ask.kind, description.photographic, ask.sample);
    Ok(Outcome::Made(Box::new(Made {
        bytes: picture.bytes,
        media_type: picture.media_type,
        width,
        height,
        description,
        label,
        prompt: prompt.to_owned(),
        model: picture.model,
        seed: Some(seed),
        verdict,
        cost_micro_usd: picture.cost_micro_usd,
    })))
}

/// `WIDTHxHEIGHT` as numbers.
pub fn dimensions(size: &str) -> (Option<u32>, Option<u32>) {
    match size.split_once('x') {
        Some((w, h)) => (w.trim().parse().ok(), h.trim().parse().ok()),
        None => (None, None),
    }
}
