//! Routing (design §4.3): purposes, tiers, providers, specialists and prices.
//!
//! - **Callers never name a model or a provider.** They name a purpose and, optionally, a tier
//!   (`x-agentrix-tier: fast|standard|deep|auto`).
//! - **Each tier maps to one model per provider,** and `AGENTRIX_PROVIDER` picks the provider
//!   for every tier at once (the S3-10 switch).
//! - **Specialist inference Claude doesn't offer stays on Vultr whatever the provider setting:**
//!   embeddings and rerank (and the vector store). `screen` and `decide` follow the provider:
//!   Claude Haiku in Phase 1, then Vultr's safety model (nemotron) and systemone (clef) after the
//!   switch (James, 7 Nov: Claude until validated; Vultr for what Claude lacks).
//! - `AGENTRIX_ROUTE_<PURPOSE>=provider:model` overrides one purpose.
//!
//! Every variable has the design's default in code, so a missing variable still works.

use std::collections::HashMap;

/// Who serves a call (R11). A provider change is a routing change, never a silent failover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Vultr,
    Anthropic,
}

impl Provider {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "vultr" => Some(Self::Vultr),
            "anthropic" => Some(Self::Anthropic),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vultr => "vultr",
            Self::Anthropic => "anthropic",
        }
    }

    fn env_suffix(self) -> &'static str {
        match self {
            Self::Vultr => "VULTR",
            Self::Anthropic => "ANTHROPIC",
        }
    }
}

/// How much model a call gets, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Fast,
    Standard,
    Deep,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Fast, Tier::Standard, Tier::Deep];

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "fast" => Some(Self::Fast),
            "standard" => Some(Self::Standard),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Standard => "standard",
            Self::Deep => "deep",
        }
    }

    pub fn up(self) -> Self {
        match self {
            Self::Fast => Self::Standard,
            Self::Standard | Self::Deep => Self::Deep,
        }
    }

    pub fn down(self) -> Self {
        match self {
            Self::Deep => Self::Standard,
            Self::Standard | Self::Fast => Self::Fast,
        }
    }

    fn env_name(self) -> &'static str {
        match self {
            Self::Fast => "FAST",
            Self::Standard => "STANDARD",
            Self::Deep => "DEEP",
        }
    }

    /// Design §4.3's default model for this tier on `provider`.
    fn default_model(self, provider: Provider) -> &'static str {
        match (provider, self) {
            (Provider::Anthropic, Tier::Deep) => "claude-opus-5-5",
            (Provider::Anthropic, Tier::Standard) => "claude-sonnet-5-5",
            (Provider::Anthropic, Tier::Fast) => "claude-haiku-5-5",
            (Provider::Vultr, Tier::Deep) => "glm-5.3",
            (Provider::Vultr, Tier::Standard) => "qwen3.8-27b",
            (Provider::Vultr, Tier::Fast) => "deepseek-v4.1-flash-1017",
        }
    }
}

/// Why a caller wants a model. Every call names one in `x-agentrix-purpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Build,
    Concierge,
    Brief,
    VisionParity,
    Screen,
    Decide,
    Embed,
    Rerank,
    /// The gateway's own tier router (step 2 of dynamic selection). Callers cannot name it.
    Router,
    /// Images: generate, edit, screen and describe (02 §6; W11). Specialists on Vultr.
    Image,
    /// Speech to text from a recording (`whisper-large-v3-turbo`).
    VoiceListen,
    /// Speech to text while the owner speaks, over the realtime WebSocket (`voxtral`).
    VoiceLive,
    /// Text to speech (`qwen3-tts`).
    VoiceSpeak,
    /// Agentrix SaaS: negotiating a debt plan or a dispute with a counterparty (Claude Sonnet
    /// under the default provider).
    Negotiate,
    /// Agentrix SaaS: an agent's routine turn, collecting, paying, reminding (Claude Haiku under
    /// the default provider).
    Collect,
    /// Agentrix SaaS: extracting fields from an invoice, a bill or an email. Always on Vultr.
    Extract,
}

pub const ALL_PURPOSES: [Purpose; 15] = [
    Purpose::Build,
    Purpose::Concierge,
    Purpose::Brief,
    Purpose::VisionParity,
    Purpose::Screen,
    Purpose::Decide,
    Purpose::Embed,
    Purpose::Rerank,
    Purpose::Image,
    Purpose::VoiceListen,
    Purpose::VoiceLive,
    Purpose::VoiceSpeak,
    Purpose::Negotiate,
    Purpose::Collect,
    Purpose::Extract,
];

/// A purpose's place in design §4.3's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub default_tier: Tier,
    /// Whether `x-agentrix-tier: auto` selects a tier per request.
    pub dynamic: bool,
    pub floor: Tier,
    pub ceiling: Tier,
    /// Effort on Anthropic, and `reasoning_effort` on Vultr's chat models.
    pub effort: &'static str,
}

impl Purpose {
    pub fn parse(text: &str) -> Option<Self> {
        ALL_PURPOSES.into_iter().find(|p| p.as_str() == text)
    }

    /// As stored in `usage.purpose` and used in recording paths.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Concierge => "concierge",
            Self::Brief => "brief",
            Self::VisionParity => "vision-parity",
            Self::Screen => "screen",
            Self::Decide => "decide",
            Self::Embed => "embed",
            Self::Rerank => "rerank",
            Self::Router => "router",
            Self::Image => "image",
            Self::VoiceListen => "voice.listen",
            Self::VoiceLive => "voice.live",
            Self::VoiceSpeak => "voice.speak",
            Self::Negotiate => "negotiate",
            Self::Collect => "collect",
            Self::Extract => "extract",
        }
    }

    /// Whether this purpose makes or reads images or audio, metered by units, not tokens.
    pub fn is_media(self) -> bool {
        matches!(
            self,
            Self::Image | Self::VoiceListen | Self::VoiceLive | Self::VoiceSpeak
        )
    }

    /// Design §4.3's table.
    pub fn profile(self) -> Profile {
        let p = |default_tier, dynamic, floor, ceiling, effort| Profile {
            default_tier,
            dynamic,
            floor,
            ceiling,
            effort,
        };
        match self {
            Self::Build => p(Tier::Standard, true, Tier::Standard, Tier::Deep, "high"),
            Self::Concierge => p(Tier::Standard, true, Tier::Fast, Tier::Standard, "medium"),
            Self::Brief => p(
                Tier::Standard,
                false,
                Tier::Standard,
                Tier::Standard,
                "medium",
            ),
            Self::VisionParity => p(Tier::Fast, false, Tier::Fast, Tier::Standard, "low"),
            // Fast is the draft model, standard the detailed one that also edits.
            Self::Image => p(Tier::Fast, false, Tier::Fast, Tier::Standard, "low"),
            Self::Negotiate => p(Tier::Standard, false, Tier::Standard, Tier::Deep, "medium"),
            Self::Collect => p(Tier::Fast, false, Tier::Fast, Tier::Standard, "low"),
            Self::Extract => p(Tier::Standard, false, Tier::Fast, Tier::Standard, "low"),
            Self::Screen
            | Self::Decide
            | Self::Embed
            | Self::Rerank
            | Self::Router
            | Self::VoiceListen
            | Self::VoiceLive
            | Self::VoiceSpeak => p(Tier::Fast, false, Tier::Fast, Tier::Fast, "low"),
        }
    }

    /// `max_tokens` when the caller sets none: room for the answer plus reasoning, only where
    /// an answer needs it (D10d).
    pub fn default_max_tokens(self) -> Option<u32> {
        match self {
            Self::Build => Some(16_384),
            Self::Concierge | Self::Brief => Some(4_096),
            Self::VisionParity => Some(1_024),
            Self::Screen => Some(512),
            Self::Router => Some(256),
            Self::Negotiate | Self::Extract => Some(4_096),
            Self::Collect => Some(2_048),
            Self::Decide | Self::Embed | Self::Rerank => None,
            Self::Image | Self::VoiceListen | Self::VoiceLive | Self::VoiceSpeak => None,
        }
    }

    /// The environment variable that overrides this purpose, for example
    /// `AGENTRIX_ROUTE_VISION_PARITY`.
    pub fn override_var(self) -> String {
        format!(
            "AGENTRIX_ROUTE_{}",
            self.as_str().to_ascii_uppercase().replace(['-', '.'], "_")
        )
    }
}

/// Which endpoint a call uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Chat,
    Embeddings,
    SystemOne,
    Rerank,
    /// `images/generations`; edits go to `images/edits` (W11).
    Images,
    /// `audio/transcriptions`.
    Transcriptions,
    /// `realtime?intent=transcription`, a WebSocket.
    Realtime,
    /// `audio/speech`.
    Speech,
}

impl Endpoint {
    /// The path under Vultr's base URL.
    pub fn path(self) -> &'static str {
        match self {
            Self::Chat => "chat/completions",
            Self::Embeddings => "embeddings",
            Self::SystemOne => "systemone",
            Self::Rerank => "rerank",
            Self::Images => "images/generations",
            Self::Transcriptions => "audio/transcriptions",
            Self::Realtime => "realtime",
            Self::Speech => "audio/speech",
        }
    }
}

/// The decision models Vultr's `/v1/decide` may name; `clef-flash` is for bulk decisions.
pub const DECISION_MODELS: [&str; 4] = ["clef", "clef-flash", "d3", "pplx-decider-v1.2"];

/// Anthropic's current models (Claude API reference, 7 Nov 2026).
pub const ANTHROPIC_MODELS: [&str; 3] =
    ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"];

/// Vultr's chat models that take images and call tools (S0-05).
pub const VULTR_CHAT_MODELS: [&str; 12] = [
    "glm-5.3",
    "glm-5.2",
    "glm-5.3-flash",
    "qwen3.8-27b",
    "qwen3.8-flash-next",
    "deepseek-v4.1-flash-1017",
    "minimax-m3.1-flash",
    "mimo-v2.6-flash",
    "mimo-v2.6-pro",
    "muse-glimmer-30b",
    "nemotron-3-nano-omni-30b-a3b-reasoning",
    "raon-1-mini-preview",
];

/// Vultr's image models (`GET /v1/images/models`, 8 Nov 2026). Only `qwen-image-2.1` and
/// `minimax-h3` take reference images, so only they edit.
pub const IMAGE_MODELS: [&str; 4] = [
    "z-image-turbo",
    "qwen-image-2.1",
    "moss-image",
    "minimax-h3",
];

/// Vultr's speech-to-text models for recordings.
pub const LISTEN_MODELS: [&str; 3] = [
    "whisper-large-v3-turbo",
    "qwen3-asr-1.7b",
    "granite-speech-5.0-2b",
];

/// Vultr's realtime speech-to-text models (`/v1/realtime`).
pub const LIVE_MODELS: [&str; 1] = ["voxtral-mini-4b-realtime"];

/// Vultr's text-to-speech models.
pub const SPEAK_MODELS: [&str; 4] = ["qwen3-tts-12hz-1.7b", "chatterbox", "auk", "auk-flash"];

/// One routed call: who serves it, which model, which tier, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub provider: Provider,
    pub model: String,
    pub endpoint: Endpoint,
    /// `None` for specialists, which have no tiers.
    pub tier: Option<Tier>,
    pub reason: String,
}

/// The routing configuration, read once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub provider: Provider,
    tiers: HashMap<(Tier, Provider), String>,
    embed: String,
    rerank: String,
    screen_vultr: String,
    decide_vultr: String,
    media: Media,
    overrides: HashMap<Purpose, (Provider, String)>,
}

/// The image and speech specialists (the first release design §4.3's `AGENTRIX_IMAGE`, `_IMAGE_FAST`,
/// `_STT`, `_STT_REALTIME` and `_TTS`). Always on Vultr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    /// Detailed images and edits.
    pub image: String,
    /// Quick drafts.
    pub image_fast: String,
    pub listen: String,
    pub live: String,
    pub speak: String,
}

impl Routing {
    /// Design §4.3's defaults: Phase 1 on Anthropic.
    #[cfg(test)]
    pub fn defaults() -> Self {
        Self::from_lookup(|_| None).expect("the defaults are valid")
    }

    /// The same defaults with `provider` for every tier.
    #[cfg(test)]
    pub fn on(provider: Provider) -> Self {
        let mut routing = Self::defaults();
        routing.provider = provider;
        routing
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads every variable through `lookup`, falling back to the design's defaults, and
    /// refuses a model that cannot serve where it is put.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let get = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let provider = match get("AGENTRIX_PROVIDER") {
            None => Provider::Anthropic,
            Some(text) => Provider::parse(&text).ok_or_else(|| {
                anyhow::anyhow!("AGENTRIX_PROVIDER={text}: use anthropic or vultr")
            })?,
        };
        let mut tiers = HashMap::new();
        for tier in Tier::ALL {
            for p in [Provider::Anthropic, Provider::Vultr] {
                let var = format!("AGENTRIX_TIER_{}_{}", tier.env_name(), p.env_suffix());
                let model = get(&var).unwrap_or_else(|| tier.default_model(p).to_owned());
                anyhow::ensure!(
                    chat_model(p, &model),
                    "{var}={model}: not a {} chat model",
                    p.as_str()
                );
                tiers.insert((tier, p), model);
            }
        }
        let specialist = |var: &str, default: &str, allowed: &[&str]| -> anyhow::Result<String> {
            let value = get(var).unwrap_or_else(|| default.to_owned());
            let model = value.strip_prefix("vultr:").unwrap_or(&value).to_owned();
            anyhow::ensure!(
                !value.contains(':') || value.starts_with("vultr:"),
                "{var}={value}: specialists stay on Vultr"
            );
            anyhow::ensure!(
                allowed.contains(&model.as_str()),
                "{var}={value}: not one of {allowed:?}"
            );
            Ok(model)
        };
        let embed = specialist(
            "AGENTRIX_EMBED",
            "vultr:qwen3-embedding-4b",
            &["qwen3-embedding-4b"],
        )?;
        let rerank = specialist(
            "AGENTRIX_RERANK",
            "vultr:bge-reranker-v2-m3",
            &["bge-reranker-v2-m3"],
        )?;
        let screen_vultr = specialist(
            "AGENTRIX_SCREEN_VULTR",
            "nemotron-3.5-content-safety",
            &["nemotron-3.5-content-safety"],
        )?;
        let decide_vultr = specialist("AGENTRIX_DECIDE_VULTR", "clef", &DECISION_MODELS)?;
        let media = Media {
            image: specialist("AGENTRIX_IMAGE", "vultr:qwen-image-2.1", &IMAGE_MODELS)?,
            image_fast: specialist("AGENTRIX_IMAGE_FAST", "vultr:z-image-turbo", &IMAGE_MODELS)?,
            listen: specialist(
                "AGENTRIX_STT",
                "vultr:whisper-large-v3-turbo",
                &LISTEN_MODELS,
            )?,
            live: specialist(
                "AGENTRIX_STT_REALTIME",
                "vultr:voxtral-mini-4b-realtime",
                &LIVE_MODELS,
            )?,
            speak: specialist("AGENTRIX_TTS", "vultr:qwen3-tts-12hz-1.7b", &SPEAK_MODELS)?,
        };
        let mut overrides = HashMap::new();
        for purpose in ALL_PURPOSES {
            let var = purpose.override_var();
            let Some(value) = get(&var) else {
                continue;
            };
            let (provider_text, model) = value
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("{var}={value}: write provider:model"))?;
            let p = Provider::parse(provider_text).ok_or_else(|| {
                anyhow::anyhow!("{var}={value}: unknown provider {provider_text:?}")
            })?;
            anyhow::ensure!(
                serves(purpose, p, model),
                "{var}={value}: {} has no model {model} that can serve {}",
                p.as_str(),
                purpose.as_str()
            );
            overrides.insert(purpose, (p, model.to_owned()));
        }
        Ok(Self {
            provider,
            tiers,
            embed,
            rerank,
            screen_vultr,
            decide_vultr,
            media,
            overrides,
        })
    }

    /// The model for `tier` on `provider`.
    pub fn tier_model(&self, tier: Tier, provider: Provider) -> &str {
        self.tiers
            .get(&(tier, provider))
            .map_or_else(|| tier.default_model(provider), String::as_str)
    }

    /// Routes one call to `purpose` at `tier` (already clamped by the caller of this function).
    pub fn choose(&self, purpose: Purpose, tier: Tier, reason: impl Into<String>) -> Choice {
        let reason = reason.into();
        if let Some((provider, model)) = self.overrides.get(&purpose) {
            return Choice {
                provider: *provider,
                model: model.clone(),
                endpoint: endpoint_for(purpose, *provider),
                tier: Some(tier),
                reason: format!("{reason}; {} override", purpose.override_var()),
            };
        }
        let specialist = |model: &str, endpoint| Choice {
            provider: Provider::Vultr,
            model: model.to_owned(),
            endpoint,
            tier: None,
            reason: "specialist on Vultr".to_owned(),
        };
        match (purpose, self.provider) {
            (Purpose::Embed, _) => specialist(&self.embed, Endpoint::Embeddings),
            (Purpose::Rerank, _) => specialist(&self.rerank, Endpoint::Rerank),
            (Purpose::Screen, Provider::Vultr) => specialist(&self.screen_vultr, Endpoint::Chat),
            (Purpose::Decide, Provider::Vultr) => {
                specialist(&self.decide_vultr, Endpoint::SystemOne)
            }
            (Purpose::Image, _) if tier >= Tier::Standard => Choice {
                reason: "specialist on Vultr: detailed image".to_owned(),
                ..specialist(&self.media.image, Endpoint::Images)
            },
            (Purpose::Image, _) => Choice {
                reason: "specialist on Vultr: draft image".to_owned(),
                ..specialist(&self.media.image_fast, Endpoint::Images)
            },
            (Purpose::VoiceListen, _) => specialist(&self.media.listen, Endpoint::Transcriptions),
            (Purpose::VoiceLive, _) => specialist(&self.media.live, Endpoint::Realtime),
            (Purpose::VoiceSpeak, _) => specialist(&self.media.speak, Endpoint::Speech),
            // Extraction reads documents from outside the company: Vultr's chat model, whatever
            // the provider setting.
            (Purpose::Extract, _) => Choice {
                provider: Provider::Vultr,
                model: self.tier_model(tier, Provider::Vultr).to_owned(),
                endpoint: Endpoint::Chat,
                tier: Some(tier),
                reason: format!("{reason}; extraction stays on Vultr"),
            },
            (_, provider) => Choice {
                provider,
                model: self.tier_model(tier, provider).to_owned(),
                endpoint: Endpoint::Chat,
                tier: Some(tier),
                reason,
            },
        }
    }

    /// Whether any purpose could route to `provider`.
    pub fn uses(&self, provider: Provider) -> bool {
        provider == Provider::Vultr
            || self.provider == provider
            || self.overrides.values().any(|(p, _)| *p == provider)
    }

    /// The default route of every purpose, for `/v1/status` and the startup log.
    pub fn table(&self) -> Vec<(Purpose, Choice)> {
        ALL_PURPOSES
            .into_iter()
            .map(|p| (p, self.choose(p, p.profile().default_tier, "default tier")))
            .collect()
    }
}

fn chat_model(provider: Provider, model: &str) -> bool {
    match provider {
        Provider::Anthropic => ANTHROPIC_MODELS.contains(&model),
        Provider::Vultr => VULTR_CHAT_MODELS.contains(&model),
    }
}

/// Whether an override may send `purpose` to `provider`'s `model`.
fn serves(purpose: Purpose, provider: Provider, model: &str) -> bool {
    match (purpose, provider) {
        (Purpose::Embed, Provider::Vultr) => model == "qwen3-embedding-4b",
        (Purpose::Rerank, Provider::Vultr) => model == "bge-reranker-v2-m3",
        (Purpose::Embed | Purpose::Rerank, Provider::Anthropic) => false,
        (Purpose::Screen, Provider::Vultr) => model == "nemotron-3.5-content-safety",
        (Purpose::Decide, Provider::Vultr) => DECISION_MODELS.contains(&model),
        (Purpose::Image, Provider::Vultr) => IMAGE_MODELS.contains(&model),
        (Purpose::VoiceListen, Provider::Vultr) => LISTEN_MODELS.contains(&model),
        (Purpose::VoiceLive, Provider::Vultr) => LIVE_MODELS.contains(&model),
        (Purpose::VoiceSpeak, Provider::Vultr) => SPEAK_MODELS.contains(&model),
        (p, Provider::Anthropic) if p.is_media() => false,
        (_, p) => chat_model(p, model),
    }
}

fn endpoint_for(purpose: Purpose, provider: Provider) -> Endpoint {
    match (purpose, provider) {
        (Purpose::Embed, _) => Endpoint::Embeddings,
        (Purpose::Rerank, _) => Endpoint::Rerank,
        (Purpose::Decide, Provider::Vultr) => Endpoint::SystemOne,
        (Purpose::Image, _) => Endpoint::Images,
        (Purpose::VoiceListen, _) => Endpoint::Transcriptions,
        (Purpose::VoiceLive, _) => Endpoint::Realtime,
        (Purpose::VoiceSpeak, _) => Endpoint::Speech,
        _ => Endpoint::Chat,
    }
}

/// Clamps a tier to a purpose's floor and ceiling.
pub fn clamp(purpose: Purpose, tier: Tier) -> Tier {
    let profile = purpose.profile();
    tier.max(profile.floor).min(profile.ceiling)
}

/// List prices in nano-USD per token (input, output): Vultr's from `GET /v1/models` and
/// Anthropic's from the Claude API reference, both 7 Nov 2026. Decision, embedding and rerank
/// models bill input tokens only.
fn price(model: &str, tokens_in: u64) -> Option<(u64, u64)> {
    Some(match model {
        // Claude Haiku 5.5 costs $0.50 / $2.50 per million beyond 100K prompt tokens.
        "claude-haiku-5-5" if tokens_in > 100_000 => (500, 2_500),
        "claude-haiku-5-5" => (100, 500),
        "claude-sonnet-5-5" => (2_000, 10_000),
        "claude-opus-5-5" => (4_000, 20_000),
        "glm-5.3" | "glm-5.2" => (750, 3_000),
        "glm-5.3-flash" => (100, 350),
        "qwen3.8-27b" => (150, 1_000),
        "qwen3.8-flash-next" => (100, 200),
        "deepseek-v4.1-flash-1017" => (150, 600),
        "minimax-m3.1-flash" => (200, 900),
        "mimo-v2.6-flash" => (100, 250),
        "mimo-v2.6-pro" => (400, 800),
        "muse-glimmer-30b" => (250, 1_000),
        "nemotron-3-nano-omni-30b-a3b-reasoning" => (100, 250),
        "raon-1-mini-preview" => (100, 300),
        "nemotron-3.5-content-safety" => (50, 150),
        "clef" | "d3" | "pplx-decider-v1.2" => (100, 0),
        "clef-flash" => (50, 0),
        "qwen3-embedding-4b" => (50, 0),
        // Vultr's vector store bills its embeddings but does not name its model: an estimate
        // at the embedding model's price.
        "vultr-vector-store" => (50, 0),
        "bge-reranker-v2-m3" => (50, 0),
        _ => return None,
    })
}

/// How a media model bills (Vultr's catalogue, `GET /v1/models`, 8 Nov 2026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPrice {
    /// Nano-USD per output megapixel. Edits bill like generations of the same size.
    PerMegapixel(u64),
    /// Nano-USD per second of audio, rounded up to whole seconds.
    PerSecond(u64),
}

/// The list price of an image or speech model. Models free during Vultr's evaluation
/// (`qwen-image-2.1`, `auk`, `auk-flash`) cost 0 until Vultr prices them.
pub fn media_price(model: &str) -> Option<MediaPrice> {
    Some(match model {
        "z-image-turbo" | "moss-image" => MediaPrice::PerMegapixel(20_000_000),
        // minimax-h3's cheapest tier; dearer tiers multiply the megapixels Vultr reports.
        "minimax-h3" => MediaPrice::PerMegapixel(20_000_000),
        "qwen-image-2.1" => MediaPrice::PerMegapixel(0),
        "whisper-large-v3-turbo" => MediaPrice::PerSecond(11_100),
        "qwen3-asr-1.7b" => MediaPrice::PerSecond(7_500),
        "granite-speech-5.0-2b" => MediaPrice::PerSecond(5_550),
        "voxtral-mini-4b-realtime" => MediaPrice::PerSecond(133_000),
        "qwen3-tts-12hz-1.7b" | "chatterbox" => MediaPrice::PerSecond(280_000),
        "auk" | "auk-flash" => MediaPrice::PerSecond(0),
        _ => return None,
    })
}

/// The estimated cost of `amount` (megapixels or seconds, as the model bills) in micro-USD,
/// rounded up. Seconds are rounded up to whole seconds first, as Vultr bills them.
pub fn media_cost_micro_usd(model: &str, amount: f64) -> i64 {
    let Some(price) = media_price(model) else {
        tracing::warn!(
            model,
            "no price for media model; usage recorded at zero cost"
        );
        return 0;
    };
    if !amount.is_finite() || amount <= 0.0 {
        return 0;
    }
    let nano = match price {
        MediaPrice::PerMegapixel(nano) => (amount * nano as f64).ceil(),
        MediaPrice::PerSecond(nano) => amount.ceil() * nano as f64,
    };
    ((nano / 1_000.0).ceil() as i64).max(0)
}

/// The estimated cost of one call in micro-USD, rounded up. Reasoning and thinking tokens are
/// billed as output and are already inside `tokens_out`. An unpriced model costs 0 and is
/// logged.
pub fn cost_micro_usd(model: &str, tokens_in: u64, tokens_out: u64) -> i64 {
    let Some((input, output)) = price(model, tokens_in) else {
        tracing::warn!(model, "no price for model; usage recorded at zero cost");
        return 0;
    };
    let nano = tokens_in
        .saturating_mul(input)
        .saturating_add(tokens_out.saturating_mul(output));
    i64::try_from(nano.div_ceil(1_000)).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn saas_purposes_route_to_the_model_mix() {
        let routing = Routing::from_lookup(|_| None).unwrap();
        let pick = |p: Purpose| routing.choose(p, p.profile().default_tier, "t");
        assert_eq!(pick(Purpose::Negotiate).model, "claude-sonnet-5-5");
        assert_eq!(pick(Purpose::Collect).model, "claude-haiku-5-5");
        let extract = pick(Purpose::Extract);
        assert_eq!(extract.provider, Provider::Vultr);
        assert_eq!(extract.endpoint, Endpoint::Chat);
        for p in [Purpose::Negotiate, Purpose::Collect, Purpose::Extract] {
            assert_eq!(Purpose::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn phase_one_defaults_put_tiers_on_claude_and_specialists_on_vultr() {
        let routing = Routing::defaults();
        assert_eq!(routing.provider, Provider::Anthropic);
        let build = routing.choose(Purpose::Build, Tier::Standard, "default tier");
        assert_eq!(
            (build.provider, build.model.as_str()),
            (Provider::Anthropic, "claude-sonnet-5-5")
        );
        assert_eq!(
            routing.choose(Purpose::Build, Tier::Deep, "x").model,
            "claude-opus-5-5"
        );
        assert_eq!(
            routing.choose(Purpose::Screen, Tier::Fast, "x").model,
            "claude-haiku-5-5"
        );
        let embed = routing.choose(Purpose::Embed, Tier::Fast, "x");
        assert_eq!(
            (embed.provider, embed.model.as_str(), embed.tier),
            (Provider::Vultr, "qwen3-embedding-4b", None)
        );
    }

    #[test]
    fn one_variable_flips_every_tier_to_vultr() {
        let routing = Routing::from_lookup(env(&[("AGENTRIX_PROVIDER", "vultr")])).unwrap();
        assert_eq!(
            routing.choose(Purpose::Build, Tier::Deep, "x").model,
            "glm-5.3"
        );
        assert_eq!(
            routing
                .choose(Purpose::Concierge, Tier::Standard, "x")
                .model,
            "qwen3.8-27b"
        );
        assert_eq!(
            routing.choose(Purpose::VisionParity, Tier::Fast, "x").model,
            "deepseek-v4.1-flash-1017"
        );
        let screen = routing.choose(Purpose::Screen, Tier::Fast, "x");
        assert_eq!(
            (screen.model.as_str(), screen.endpoint),
            ("nemotron-3.5-content-safety", Endpoint::Chat)
        );
        let decide = routing.choose(Purpose::Decide, Tier::Fast, "x");
        assert_eq!(
            (decide.model.as_str(), decide.endpoint),
            ("clef", Endpoint::SystemOne)
        );
    }

    #[test]
    fn overrides_and_tier_models_are_checked() {
        let routing = Routing::from_lookup(env(&[
            ("AGENTRIX_TIER_STANDARD_VULTR", "deepseek-v4.1-flash-1017"),
            ("AGENTRIX_ROUTE_BUILD", "vultr:deepseek-v4.1-flash-1017"),
        ]))
        .unwrap();
        let build = routing.choose(Purpose::Build, Tier::Deep, "rule");
        assert_eq!(
            (build.provider, build.model.as_str()),
            (Provider::Vultr, "deepseek-v4.1-flash-1017")
        );
        assert!(build.reason.contains("AGENTRIX_ROUTE_BUILD"));
        assert_eq!(
            routing.tier_model(Tier::Standard, Provider::Vultr),
            "deepseek-v4.1-flash-1017"
        );

        for bad in [
            ("AGENTRIX_PROVIDER", "openai"),
            ("AGENTRIX_TIER_DEEP_ANTHROPIC", "gpt-5"),
            ("AGENTRIX_TIER_FAST_VULTR", "claude-haiku-5-5"),
            ("AGENTRIX_ROUTE_BUILD", "gpt-4o"),
            ("AGENTRIX_ROUTE_EMBED", "anthropic:claude-haiku-5-5"),
            ("AGENTRIX_ROUTE_SCREEN", "vultr:glm-5.3"),
            ("AGENTRIX_EMBED", "anthropic:claude-haiku-5-5"),
            ("AGENTRIX_DECIDE_VULTR", "qwen3.8-27b"),
        ] {
            assert!(
                Routing::from_lookup(env(&[bad])).is_err(),
                "{bad:?} accepted"
            );
        }
    }

    #[test]
    fn tiers_clamp_to_each_purpose() {
        assert_eq!(clamp(Purpose::Build, Tier::Fast), Tier::Standard);
        assert_eq!(clamp(Purpose::Concierge, Tier::Deep), Tier::Standard);
        assert_eq!(clamp(Purpose::Screen, Tier::Deep), Tier::Fast);
        assert_eq!(Tier::Fast.up().up().up(), Tier::Deep);
    }

    #[test]
    fn every_routable_model_is_priced() {
        for model in VULTR_CHAT_MODELS
            .into_iter()
            .chain(DECISION_MODELS)
            .chain(ANTHROPIC_MODELS)
        {
            assert!(price(model, 0).is_some(), "{model} is unpriced");
        }
        // 1,000 input and 500 output tokens on glm-5.3: 750,000 + 1,500,000 nano-USD.
        assert_eq!(cost_micro_usd("glm-5.3", 1_000, 500), 2_250);
        assert_eq!(cost_micro_usd("qwen3-embedding-4b", 8, 0), 1);
        assert_eq!(cost_micro_usd("claude-haiku-5-5", 10_000, 1_000), 1_500);
        assert_eq!(cost_micro_usd("claude-haiku-5-5", 200_000, 1_000), 102_500);
        assert_eq!(cost_micro_usd("unknown-model", 1_000, 1_000), 0);
    }

    #[test]
    fn image_and_voice_route_to_vultr_specialists_and_are_priced() {
        for routing in [Routing::defaults(), Routing::on(Provider::Vultr)] {
            let draft = routing.choose(Purpose::Image, Tier::Fast, "x");
            assert_eq!(
                (
                    draft.provider,
                    draft.model.as_str(),
                    draft.endpoint,
                    draft.tier
                ),
                (Provider::Vultr, "z-image-turbo", Endpoint::Images, None)
            );
            assert_eq!(
                routing.choose(Purpose::Image, Tier::Standard, "x").model,
                "qwen-image-2.1"
            );
            assert_eq!(
                routing.choose(Purpose::VoiceListen, Tier::Fast, "x").model,
                "whisper-large-v3-turbo"
            );
            let live = routing.choose(Purpose::VoiceLive, Tier::Fast, "x");
            assert_eq!(
                (live.model.as_str(), live.endpoint),
                ("voxtral-mini-4b-realtime", Endpoint::Realtime)
            );
            assert_eq!(
                routing.choose(Purpose::VoiceSpeak, Tier::Fast, "x").model,
                "qwen3-tts-12hz-1.7b"
            );
        }
        assert_eq!(Purpose::parse("voice.speak"), Some(Purpose::VoiceSpeak));
        assert_eq!(
            Purpose::VoiceSpeak.override_var(),
            "AGENTRIX_ROUTE_VOICE_SPEAK"
        );
        let routing = Routing::from_lookup(env(&[
            ("AGENTRIX_IMAGE_FAST", "vultr:moss-image"),
            ("AGENTRIX_ROUTE_VOICE_LISTEN", "vultr:qwen3-asr-1.7b"),
        ]))
        .unwrap();
        assert_eq!(
            routing.choose(Purpose::Image, Tier::Fast, "x").model,
            "moss-image"
        );
        let listen = routing.choose(Purpose::VoiceListen, Tier::Fast, "x");
        assert_eq!(
            (listen.model.as_str(), listen.endpoint),
            ("qwen3-asr-1.7b", Endpoint::Transcriptions)
        );
        for bad in [
            ("AGENTRIX_IMAGE", "anthropic:claude-opus-5-5"),
            ("AGENTRIX_TTS", "vultr:glm-5.3"),
            ("AGENTRIX_ROUTE_IMAGE", "anthropic:claude-opus-5-5"),
            ("AGENTRIX_ROUTE_VOICE_SPEAK", "vultr:whisper-large-v3-turbo"),
        ] {
            assert!(
                Routing::from_lookup(env(&[bad])).is_err(),
                "{bad:?} accepted"
            );
        }
        for model in IMAGE_MODELS
            .into_iter()
            .chain(LISTEN_MODELS)
            .chain(LIVE_MODELS)
            .chain(SPEAK_MODELS)
        {
            assert!(media_price(model).is_some(), "{model} is unpriced");
        }
        // 1.05 megapixels on z-image-turbo at $0.02 each.
        assert_eq!(media_cost_micro_usd("z-image-turbo", 1.048_576), 20_972);
        assert_eq!(media_cost_micro_usd("qwen-image-2.1", 4.0), 0);
        // 4.12 s of realtime audio bills 5 s at $0.000133.
        assert_eq!(media_cost_micro_usd("voxtral-mini-4b-realtime", 4.12), 665);
        assert_eq!(media_cost_micro_usd("qwen3-tts-12hz-1.7b", 3.0), 840);
        assert_eq!(media_cost_micro_usd("unknown", 3.0), 0);
    }
}
