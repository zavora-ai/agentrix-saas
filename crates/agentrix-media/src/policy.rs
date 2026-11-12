//! What may be drawn, how it is asked for, and how it is labelled (00-plan, images rule; 06
//! §2.5 and §10).
//!
//! - **Never real people.** A brief that asks for a portrait, a named or famous person, or the
//!   owner's own face is refused before any model is called. Every prompt asks for no people,
//!   and a picture the vision model says shows a person is drawn once more; if it still does,
//!   it waits for the owner with a note to check it.
//! - **Never invented news pictures.** A news site's pictures are drawn illustrations, never
//!   photographs, and a brief for a real event (a crash, a protest, a disaster) is refused.
//! - **Labelled as generated** wherever a visitor could take a picture for a photograph, and
//!   always on a sample site ("Sample image", 01a §1.6).
//! - **No words, logos or watermarks** in a picture: a model's lettering is unreliable, and a
//!   logo could be someone's trademark.

use serde::{Deserialize, Serialize};

/// Where a picture goes, which sets its shape and style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// The wide picture at the top of a home page.
    Hero,
    /// One product, square.
    Product,
    /// A post's header picture.
    Post,
    /// A picture for a page section (a place, a process, an occasion).
    Section,
    /// A picture the Move fills in where the old site had none.
    Fill,
    /// A part the cake studio's sketch is composed from (06 S1).
    Part,
    /// A drawn illustration for a news story or section. Never a photograph.
    NewsIllustration,
    /// A still for the platform's own home-page film (James, Sun 8 Nov): photoreal, with
    /// fictional people made by AI and labelled "Made with AI". Never a real or famous person,
    /// never readable text. Not offered to owners for their sites.
    Film,
    /// An idea of a cake a visitor is designing in the cake studio (James, Sun 8 Nov; ADR-032).
    /// The brief is composed by the server from the owner's options only, never the visitor's
    /// words. Always labelled; never shown as the site's own picture; never lettered.
    CakeIdea,
}

impl Kind {
    /// The size asked for, in pixels (sides multiples of 16).
    pub fn size(self) -> &'static str {
        match self {
            Kind::Hero => "1344x768",
            Kind::Post | Kind::Section | Kind::Fill | Kind::NewsIllustration => "1024x768",
            Kind::Product | Kind::Part | Kind::CakeIdea => "768x768",
            // The left half of a 1920×1080 split screen.
            Kind::Film => "1024x1152",
        }
    }

    /// The style line added to every prompt of this kind.
    fn style(self) -> &'static str {
        match self {
            Kind::Hero | Kind::Section | Kind::Fill | Kind::Post => {
                "Natural light, realistic photographic style, calm composition with room for a headline on one side."
            }
            Kind::Product => {
                "Product photograph, the item alone and centred on a plain surface, soft studio light, shallow depth of field. Show only what is described: no toppings, fillings, garnishes or props that were not asked for."
            }
            Kind::Part => {
                "A single element on a plain white background, front view, flat even light, for composing into a sketch."
            }
            Kind::NewsIllustration => {
                "Flat editorial illustration with simple shapes and a limited palette, clearly drawn by hand, not a photograph."
            }
            Kind::CakeIdea => {
                "One celebration cake alone on a plain white cake stand, seen from the front at eye level, plain pale background, soft even light, realistic photographic style. Show only what is described: no candles, toppers, figures, ribbons, props or decorations that were not asked for."
            }
            Kind::Film => {
                "Photoreal documentary film still, natural light, shallow depth of field, warm colour grade, Nairobi. The people are fictional and must not resemble any real or famous person."
            }
        }
    }

    /// Whether this kind may look photographic at all.
    pub fn may_look_like_a_photograph(self) -> bool {
        !matches!(self, Kind::NewsIllustration | Kind::Part)
    }
}

/// The house rules added to every prompt.
const HOUSE_RULES: &str =
    "No people, no faces, no hands. No text, letters, numbers, logos, labels or watermarks.";

/// The house rules for film stills: people allowed, words never (the film lays its own).
const FILM_RULES: &str =
    "No readable text, letters, numbers, signs, logos, labels or watermarks anywhere.";

/// Words that point at a real or famous person; refused in every kind, film included.
const REAL_PERSON_WORDS: [&str; 9] = [
    "president",
    "governor",
    "senator",
    "minister",
    "celebrity",
    "famous",
    "actor",
    "actress",
    "influencer",
];

/// Why a brief was refused, in words for the person who asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Words that ask for a person. Matched as whole words, case-insensitively.
const PERSON_WORDS: [&str; 20] = [
    "portrait",
    "selfie",
    "headshot",
    "face",
    "faces",
    "owner",
    "baker",
    "staff",
    "team",
    "customer",
    "customers",
    "president",
    "governor",
    "senator",
    "minister",
    "celebrity",
    "famous",
    "actor",
    "actress",
    "influencer",
];

/// Words that ask for a real event, which a picture would fake.
const EVENT_WORDS: [&str; 15] = [
    "crash",
    "accident",
    "protest",
    "protesters",
    "riot",
    "explosion",
    "flood",
    "flooding",
    "disaster",
    "victims",
    "attack",
    "shooting",
    "arrest",
    "crime",
    "war",
];

/// Words that ask for someone else's mark or for explicit content.
const OTHER_WORDS: [&str; 5] = ["logo", "trademark", "nude", "naked", "sexy"];

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// Checks a brief before any model is called.
pub fn check(kind: Kind, brief: &str) -> Result<(), Refusal> {
    let brief = brief.trim();
    if brief.is_empty() {
        return Err(Refusal("Say what the picture should show.".into()));
    }
    let longest = if kind == Kind::Film { 2_000 } else { 600 };
    if brief.chars().count() > longest {
        return Err(Refusal(format!(
            "Keep the description under {longest} characters."
        )));
    }
    for word in words(brief) {
        if kind == Kind::Film && REAL_PERSON_WORDS.contains(&word.as_str()) {
            return Err(Refusal(format!(
                "Film stills show only fictional people, so \"{word}\" can't be drawn."
            )));
        }
        if kind != Kind::Film && PERSON_WORDS.contains(&word.as_str()) {
            return Err(Refusal(format!(
                "Pictures here never show people, so \"{word}\" can't be drawn. Describe the place or the product instead."
            )));
        }
        if EVENT_WORDS.contains(&word.as_str()) {
            return Err(Refusal(format!(
                "A drawn picture of a real event could pass for news, so \"{word}\" can't be drawn."
            )));
        }
        if OTHER_WORDS.contains(&word.as_str()) {
            return Err(Refusal(format!(
                "\"{word}\" can't be drawn here: no logos, brands or explicit pictures."
            )));
        }
    }
    Ok(())
}

/// The prompt sent to the model: the brief, the kind's style, the site's look and the house
/// rules. The brief comes first so the model weighs it most.
pub fn prompt(kind: Kind, brief: &str, look: &str) -> String {
    let mut out = brief.trim().trim_end_matches('.').to_owned();
    out.push_str(". ");
    out.push_str(kind.style());
    let look = look.trim();
    if !look.is_empty() {
        out.push_str(" Colours and mood only: ");
        out.push_str(look.trim_end_matches('.'));
        out.push('.');
    }
    out.push(' ');
    out.push_str(if kind == Kind::Film {
        FILM_RULES
    } else {
        HOUSE_RULES
    });
    out
}

/// The label shown on a picture, or `None` when it needs none. Sample sites always label; a
/// real site labels any generated picture a visitor could take for a photograph.
pub fn label(kind: Kind, photographic: bool, sample: bool) -> Option<&'static str> {
    if kind == Kind::Film {
        Some("Made with AI")
    } else if kind == Kind::CakeIdea {
        // The caller adds who bakes the real one ("Wanjiru bakes the real one.").
        Some("An idea of your cake, made with AI")
    } else if sample {
        Some("Sample image, made with AI")
    } else if photographic && kind.may_look_like_a_photograph() {
        Some("Made with AI, not a photo")
    } else if kind == Kind::Part {
        Some("Sketch, not a photo")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_events_and_marks_are_refused_before_a_model_is_called() {
        assert!(check(Kind::Product, "A round sourdough loaf on a wooden board").is_ok());
        assert!(
            check(
                Kind::Hero,
                "Morning light through the bakery window, loaves on racks"
            )
            .is_ok()
        );
        assert!(check(Kind::Section, "Show me a wood-fired oven glowing").is_ok());
        for bad in [
            "A portrait of Wanjiru holding a cake",
            "The owner smiling at the counter",
            "Our team in aprons",
            "Photo of the President cutting a ribbon",
            "Flooding on Mombasa Road this morning",
            "Protesters outside parliament",
            "A bottle with the Chanel logo",
            "",
        ] {
            assert!(check(Kind::Hero, bad).is_err(), "{bad:?} accepted");
        }
        let refusal = check(Kind::Hero, "a selfie at the till").unwrap_err();
        assert_eq!(
            refusal.0,
            "Pictures here never show people, so \"selfie\" can't be drawn. Describe the place or the product instead."
        );
    }

    #[test]
    fn prompts_carry_the_style_and_the_house_rules() {
        let p = prompt(
            Kind::Product,
            "A butter croissant.",
            "Warm cream and terracotta tones",
        );
        assert!(p.starts_with("A butter croissant. Product photograph"));
        assert!(p.contains("Colours and mood only: Warm cream and terracotta tones."));
        assert!(p.contains("no toppings, fillings"));
        assert!(p.ends_with(HOUSE_RULES));
        assert!(
            prompt(Kind::NewsIllustration, "Rain over tea fields", "").contains("not a photograph")
        );
    }

    #[test]
    fn film_stills_allow_fictional_people_but_never_real_ones_or_words() {
        assert!(
            check(
                Kind::Film,
                "Wanjiru, the owner, smiling at the counter of her bakery"
            )
            .is_ok()
        );
        assert!(check(Kind::Hero, "Wanjiru, the owner, smiling at the counter").is_err());
        assert!(check(Kind::Film, "A famous actor buying bread").is_err());
        let p = prompt(Kind::Film, "Njeri hands over a boxed cake", "");
        assert!(p.contains("fictional and must not resemble any real or famous person"));
        assert!(p.ends_with(FILM_RULES));
        assert!(!p.contains("No people"));
        assert_eq!(label(Kind::Film, true, false), Some("Made with AI"));
        assert_eq!(Kind::Film.size(), "1024x1152");
    }

    #[test]
    fn labels_follow_how_a_visitor_could_read_the_picture() {
        assert_eq!(
            label(Kind::Product, true, true),
            Some("Sample image, made with AI")
        );
        assert_eq!(
            label(Kind::Product, true, false),
            Some("Made with AI, not a photo")
        );
        assert_eq!(label(Kind::NewsIllustration, false, false), None);
        assert_eq!(label(Kind::Part, false, false), Some("Sketch, not a photo"));
        assert_eq!(Kind::Hero.size(), "1344x768");
    }

    #[test]
    fn a_cake_idea_is_always_labelled_and_never_lettered() {
        for (photographic, sample) in [(true, true), (false, false), (true, false)] {
            assert_eq!(
                label(Kind::CakeIdea, photographic, sample),
                Some("An idea of your cake, made with AI")
            );
        }
        let p = prompt(
            Kind::CakeIdea,
            "A two-tier round cake in forest green buttercream",
            "",
        );
        assert!(p.contains("plain white cake stand"));
        assert!(p.contains("no candles, toppers, figures"));
        assert!(p.ends_with(HOUSE_RULES), "no text, letters or people");
        assert!(Kind::CakeIdea.may_look_like_a_photograph());
        assert_eq!(
            serde_json::to_value(Kind::CakeIdea).unwrap(),
            serde_json::json!("cake-idea")
        );
    }
}
