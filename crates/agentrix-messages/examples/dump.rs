//! Renders every catalogue example to a folder, for reading and copy review:
//! `cargo run -p agentrix-messages --example dump -- <dir>`.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "messages-preview".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let mut index = String::from(
        "<!doctype html><meta charset=utf-8><title>Messages</title><body style=\"font:15px/1.5 system-ui;margin:24px\"><h1>Every message</h1>",
    );
    for def in agentrix_messages::catalogue::all() {
        let r = agentrix_messages::render(def.id, &def.example_context())?;
        let _ = write!(
            index,
            "<h3>{} <small>({:?}, {:?})</small></h3><p>{}</p><ul>",
            def.id, def.audience, def.kind, def.trigger
        );
        if let Some(e) = &r.email {
            let name = format!("{}.html", def.id);
            std::fs::write(dir.join(&name), &e.html)?;
            std::fs::write(
                dir.join(format!("{}.txt", def.id)),
                format!(
                    "From: {}\nSubject: {}\n\n{}",
                    e.from_name, e.subject, e.text
                ),
            )?;
            let _ = write!(
                index,
                "<li><a href=\"{name}\">Email</a>: {} · <a href=\"{}.txt\">text</a></li>",
                e.subject, def.id
            );
        }
        if let Some(p) = &r.push {
            let _ = write!(index, "<li>Push: <b>{}</b> {}</li>", p.title, p.body);
        }
        if let Some(c) = &r.chat {
            let _ = write!(
                index,
                "<li>Chat: <pre style=\"white-space:pre-wrap\">{}</pre>replies {:?}, link {:?}</li>",
                c.text, c.replies, c.link
            );
        }
        index.push_str("</ul>");
    }
    std::fs::write(dir.join("index.html"), index)?;
    println!("{}", dir.join("index.html").display());
    Ok(())
}
