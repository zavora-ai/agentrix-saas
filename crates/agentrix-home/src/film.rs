//! The home page film, "The same Saturday, twice": a 19-second silent cut plays in the hero,
//! and the 2 min 24 s film has its own page at `/film`, with a transcript.
//!
//! The videos are too big to compile into node (the film alone is about 14 MB), so node serves
//! them from a directory on its own disk named by `AGENTRIX_HOME_FILM_DIR`: `hero.mp4` and
//! `film.mp4`, and `hero.webm` and `film.webm` if they're there. `film/render.py` makes them.
//! Serving them from this origin keeps the page's CSP at `'self'`, and no third party learns who
//! watched. Without the directory, the hero shows its poster (compiled in) and `/film` is a 404.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use serde::Serialize;

/// The only files node serves from the film directory.
const FILES: [&str; 7] = [
    "hero.mp4",
    "hero.webm",
    "film.mp4",
    "film.webm",
    "classy.mp4",
    "tana.mp4",
    "salon.mp4",
];

/// The business-type films (`film/films/<id>.js`), offered on the Samples page when on disk.
const MORE: [(&str, &str, &str, &str); 3] = [
    (
        "classy",
        "A perfume shop",
        "The gift at eleven at night. A search that answers, a plan for five bridesmaids, and a shopper's AI assistant at the agent door.",
        "1 min 46 s",
    ),
    (
        "tana",
        "A local paper",
        "The wrong number. One correction that reaches the story, the answers and the AI assistants that quote it.",
        "1 min 53 s",
    ),
    (
        "salon",
        "A salon with a hacked site",
        "The site that sent clients to a casino. The check-up, the Move through sealed rooms, and bookings by Telegram.",
        "1 min 44 s",
    ),
];

/// The four films' stories in plain words, for the Samples desk to answer from. Each is a
/// sample story made with AI; Nyota Salon has no sample site, so its new site is an illustration.
pub const STORIES: [(&str, &str); 4] = [
    (
        "Films · The same Saturday, twice (a bakery)",
        "A sample story made with AI, 2 min 24 s, about Kilele Bakery in Westlands. Before the move: orders and a question about a custom cake go unread while the shop is busy, orders pile up at night, and Sunday morning is chaos. After the move: orders land in one list with a time to collect, a customer designs a cake from what the bakery makes, the question the site can't answer goes to the owner on Telegram, and the shop closes at six while the website keeps working.",
    ),
    (
        "Films · The gift at eleven at night (a perfume shop)",
        "A sample story made with AI, 1 min 46 s, about Classy Perfumes, a perfume and gift shop. Before the move: the shop's search finds nothing for five bridesmaids' gifts, a message waits in an inbox, and a shopper's AI assistant can't read the site and buys elsewhere. After the move: the search answers the question people ask and says what's over budget, the site drafts a gift plan from what's in stock, the shopper's AI assistant buys through the agent door while a person still pays, and the owner confirms before anything is charged.",
    ),
    (
        "Films · The wrong number (a local paper)",
        "A sample story made with AI, 1 min 53 s, about the Tana Valley Times, a local weekly. Before the move: a wrong figure goes out, AI assistants repeat it, and the fix is buried at the bottom of the story. After the move: one correction shows at the top of the story, on the corrections page, in the desks' answers and at the agent door, readers can ask Catch me up, and the Friday five goes out with the corrections.",
    ),
    (
        "Films · The site that sent clients to a casino (a hair salon)",
        "A sample story made with AI, 1 min 44 s, about Nyota Salon, a hair salon in Kilimani. Nyota is fictional and has no sample site, so its new site in the film is an illustration. Before the move: the hacked WordPress sent a client to a casino, search results warned people away, bookings dried up, and a fixer's backup brought the spam back. After the move: the free check found what the fixer missed, the move read the old site in sealed rooms with no network and no keys and left the infection behind, going live waited for the owner and could be undone for 30 days, clients can ask the site at any hour, booking requests reach the owner on Telegram, and bots knocking on the old WordPress login reach nothing.",
    ),
];

/// The film's files on disk.
#[derive(Debug, Clone)]
pub struct Film {
    dir: PathBuf,
    present: Vec<&'static str>,
    /// Changes when any file's size or modified time does: the URLs carry it as `?v=…`.
    version: String,
}

impl Film {
    /// The film in `dir`, if `hero.mp4` is there.
    pub fn open(dir: impl Into<PathBuf>) -> Option<Self> {
        let dir = dir.into();
        let mut stamp = DefaultHasher::new();
        let present: Vec<&'static str> = FILES
            .iter()
            .copied()
            .filter(|name| match std::fs::metadata(dir.join(name)) {
                Ok(meta) if meta.is_file() => {
                    (name, meta.len(), meta.modified().ok()).hash(&mut stamp);
                    true
                }
                _ => false,
            })
            .collect();
        if !present.contains(&"hero.mp4") {
            tracing::warn!(dir = %dir.display(), "home: no hero.mp4 in the film directory, so the hero shows its poster");
            return None;
        }
        Some(Self {
            dir,
            present,
            version: format!("{:012x}", stamp.finish() & 0xffff_ffff_ffff),
        })
    }

    /// The film in `AGENTRIX_HOME_FILM_DIR`, when it's set.
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var("AGENTRIX_HOME_FILM_DIR").ok()?;
        (!dir.trim().is_empty()).then_some(())?;
        Self::open(dir)
    }

    /// Where `file` is on disk, if it's one of the film's files and it's there.
    pub fn path(&self, file: &str) -> Option<PathBuf> {
        FILES
            .iter()
            .find(|f| **f == file)
            .filter(|f| self.present.contains(f))
            .map(|f| self.dir.join(f))
    }

    /// The `<source>` URLs for the hero and the film page.
    pub fn view(&self) -> FilmView {
        let has = |name: String| self.present.iter().any(|f| *f == name);
        let sources = |stem: &str| {
            has(format!("{stem}.mp4")).then(|| Sources {
                mp4: format!("/_home/film/{stem}.mp4?v={}", self.version),
                webm: has(format!("{stem}.webm"))
                    .then(|| format!("/_home/film/{stem}.webm?v={}", self.version)),
            })
        };
        FilmView {
            hero: sources("hero"),
            film: sources("film"),
            more: MORE
                .iter()
                .filter(|(id, ..)| has(format!("{id}.mp4")))
                .map(|(id, kind, line, length)| MoreFilm {
                    kind: (*kind).into(),
                    line: (*line).into(),
                    length: (*length).into(),
                    mp4: format!("/_home/film/{id}.mp4?v={}", self.version),
                })
                .collect(),
        }
    }
}

/// What the templates need to play the film.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FilmView {
    /// The hero's 19-second cut, when it's on disk.
    pub hero: Option<Sources>,
    /// The whole film, when it's on disk.
    pub film: Option<Sources>,
    /// The business-type films on disk.
    pub more: Vec<MoreFilm>,
}

/// One business-type film.
#[derive(Debug, Clone, Serialize)]
pub struct MoreFilm {
    /// "A perfume shop".
    pub kind: String,
    /// One line on what happens in it.
    pub line: String,
    /// "1 min 46 s".
    pub length: String,
    /// Where it plays.
    pub mp4: String,
}

/// One video's sources: WebM first when there is one, MP4 for every browser.
#[derive(Debug, Clone, Serialize)]
pub struct Sources {
    /// H.264.
    pub mp4: String,
    /// VP9, smaller.
    pub webm: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Film;

    fn dir_with(files: &[&str]) -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        for f in files {
            std::fs::write(dir.0.join(f), b"not really a video").expect("writes a test file");
        }
        dir
    }

    /// A directory under the system temp dir, removed on drop.
    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let path = std::env::temp_dir().join(format!(
                    "agentrix-home-film-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).expect("makes a temp dir");
                Self(path)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn without_the_hero_cut_there_is_no_film() {
        let dir = dir_with(&["film.mp4"]);
        assert!(Film::open(&dir.0).is_none());
    }

    #[test]
    fn only_the_film_files_are_served() {
        let dir = dir_with(&["hero.mp4", "hero.webm", "notes.txt"]);
        let film = Film::open(&dir.0).expect("hero.mp4 is there");
        assert!(film.path("hero.mp4").is_some());
        assert!(film.path("film.mp4").is_none(), "not on disk");
        assert!(film.path("notes.txt").is_none(), "not a film file");
        assert!(film.path("../hero.mp4").is_none());
        let view = film.view();
        let hero = view.hero.expect("the hero cut");
        assert!(hero.mp4.starts_with("/_home/film/hero.mp4?v="));
        assert!(hero.webm.is_some());
        assert!(view.film.is_none());
    }
}
