//! `agentrix-media`: images at the centre of every site (W11; 00-plan's images rule; 02 §6).
//!
//! A site without pictures looks dull, so every site gets them: hero and product pictures,
//! pictures the Move fills in where an old site had none, the parts the cake studio's sketch is
//! built from, and pictures for posts. Each one is:
//! - **asked for in words** and checked against [`policy`] before any model is called (never
//!   real people, never invented news pictures, no logos);
//! - **drawn** through the Model Gateway's `image` purpose ([`client`]); this crate never holds
//!   a model key;
//! - **screened** by the content-safety model, and dropped if it fails;
//! - **described** by a vision model for its alt text and whether a visitor could take it for a
//!   photograph, which decides its **label**;
//! - **kept by digest** in node's content store as a draft ([`library`]);
//! - **shown on a site only once the owner approves it** in the console's media library, which
//!   issues a receipt. Drafts are visible to the owner only.
//!
//! [`ideas`] are the one exception to the owner's approval: a picture of what one visitor is
//! designing (the cake studio's idea of a cake, ADR-032), composed from the owner's options,
//! screened, always labelled, and kept apart from the library so it never becomes the site's
//! own picture.

pub mod apps;
pub mod client;
pub mod fill;
pub mod ideas;
pub mod intent;
pub mod library;
pub mod pipeline;
pub mod policy;
pub mod routes;
pub mod samples;
pub mod voice;

use agentrix_host::{Mounted, NodeContext};
use axum::Router;

pub use library::{Shown, shown};

/// Mounts images and voice on node (contract C11): the media library's routes, `/media/…`,
/// the voice routes and the panel's assets on the platform host, and a task that seeds the
/// sample pictures into the content store.
///
/// Wiring: node runs migrations `1200_media.sql`, `1201_voice_notes.sql` and
/// `1202_media_ideas.sql` (node's migrator);
/// the sample hosts merge [`public_router`]; the shell gets [`apps::console_apps`] (Pictures
/// and Talk in the dock).
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    mount_with_runners(ctx, Vec::new())
}

/// [`mount`] with the decision cards' effect runners: pass node the same list it gives
/// `agentrix_decisions::mount_with`, so an "allow once" by voice runs like a tap.
pub fn mount_with_runners(
    ctx: &NodeContext,
    runners: Vec<std::sync::Arc<dyn agentrix_decisions::EffectRunner>>,
) -> anyhow::Result<Mounted> {
    let state = routes::MediaState::new(ctx).with_runners(runners);
    let seeding = state.clone();
    let task = async move {
        let ctx = seeding.ctx();
        match samples::seed(&ctx.pool, &ctx.blobs).await {
            Ok(added) => tracing::info!(added, "media: sample pictures seeded"),
            Err(err) => tracing::error!(error = %err, "media: could not seed sample pictures"),
        }
    };
    Ok(Mounted::default()
        .platform(
            routes::console_router(state.clone())
                .merge(routes::picture_router(state.clone()))
                .merge(voice::voice_router(state))
                .merge(voice::assets_router()),
        )
        .task("media: sample pictures", Box::pin(task)))
}

/// Asks for a picture from another crate (the Editor for a post, a sample's studio): checks
/// the brief, draws, screens, describes and keeps it as a draft in `slot` for the owner to
/// approve in Pictures. `asked_by` names who asked (`editor`, `studio`). Returns the drafts;
/// a picture the safety check stopped comes back as none, with its note kept.
pub async fn draft(
    ctx: &NodeContext,
    site: &str,
    slot: &str,
    kind: policy::Kind,
    brief: &str,
    asked_by: &str,
) -> Result<Vec<library::Image>, pipeline::MakeError> {
    let state = routes::MediaState::new(ctx);
    let request = library::Request {
        site,
        slot,
        kind,
        brief,
        asked_by,
        replaces: None,
    };
    let ask = pipeline::Ask {
        kind,
        brief: brief.to_owned(),
        look: String::new(),
        context: format!("A picture for {slot}"),
        n: 1,
        seed: None,
        tier: "fast",
        sample: routes::is_sample(site),
    };
    routes::draw_and_keep(&state, &request, &ask)
        .await
        .map(|(made, _blocked)| made)
}

/// `GET /media/{hex}.{ext}`: approved pictures only, for any host that shows them (the
/// sample sites merge this into their router).
pub fn public_router(ctx: &NodeContext) -> Router {
    routes::picture_router(routes::MediaState::new(ctx))
}
