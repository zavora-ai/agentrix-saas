//! The kit's gallery: every component in every theme, for comparing against `concept/`.
//!
//! ```sh
//! CARGO_TARGET_DIR=… cargo run -p agentrix-ui --example gallery
//! # http://localhost:18700/          platform page layout, light (add ?theme=dark)
//! # http://localhost:18700/shell     the OS shell (add ?night=1 for Night Time)
//! # http://localhost:18700/site/kilele   a sample site (classy, tana, kestrel, zavora)
//! # http://localhost:18700/error/404     error pages (403, 500)
//! ```
//!
//! Everything shown is sample data for review, never a real business.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query};
use axum::response::Html;
use axum::routing::get;
use minijinja::{Environment, Value, context};

const GALLERY: &str = include_str!("gallery.html");

fn env() -> Environment<'static> {
    let mut env = agentrix_ui::environment();
    env.add_template("gallery.html", GALLERY)
        .expect("the gallery parses");
    env
}

fn data(q: &HashMap<String, String>, layout: &str, host: &str) -> Value {
    let svg = Value::from_safe_string(String::new());
    // Quiet hours only on the shell, and ?day=1 turns them off to review the light shell.
    let quiet = (layout == "shell" && !q.contains_key("day"))
        .then(|| context! { from => "22:00", to => "06:00", tz => "Africa/Nairobi" });
    context! {
        layout => layout,
        theme => q.get("theme").cloned().unwrap_or_default(),
        night => q.contains_key("night"),
        quiet_hours => quiet,
        site => context! { key => host, name => match host {
            "kilele" => "Kilele Bakery", "classy" => "Classy Perfumes", "tana" => "Tana Valley Times",
            "kestrel" => "The Kestrel Post", _ => "zavora.ai" }, sample => true },
        svg => svg,
        kilele => kilele_agents(),
        team => serde_json::json!({"site": "kilele", "ink": "#2b1d14", "squareness": 4.6, "letters": "K",
            "members": [{"webagent": "cakes", "fill": "#8e3b46"}, {"webagent": "bread", "fill": "#9c5725"},
                        {"webagent": "orders", "fill": "#3b2416"}, {"webagent": "editor", "fill": "#577035"}]}),
        agents => vec![
            context! { id => "concierge", name => "Concierge", glyph => "Co", color => "#b72d50", state => "answering" },
            context! { id => "sales", name => "Sales", glyph => "Sa", color => "#0f6b62", state => "waiting" },
            context! { id => "editor", name => "Editor", glyph => "Ed", color => "#7a4b00", state => "working" },
            context! { id => "reputation", name => "Reputation", glyph => "Re", color => "#6b2f86", state => "asleep" },
            context! { id => "cashier", name => "Cashier", glyph => "Ca", color => "#11734a", state => "waiting" },
            context! { id => "guardian", name => "Guardian", glyph => "Gu", color => "#45597f", state => "listening" },
        ],
    }
}

/// Kilele's roster as the marks prototype draws it (sample data for review).
fn kilele_agents() -> Value {
    let spec = |id: &str, fill: &str, glyph: &str, seed: u32| {
        serde_json::json!({"markSpec": 1, "site": "kilele", "webagent": id,
            "silhouette": {"shape": "squircle", "squareness": 4.6},
            "color": {"fill": fill, "glyph": "#ffffff"},
            "glyph": {"kind": "role", "id": glyph}, "signature": {"family": "grain", "seed": seed}})
    };
    Value::from_serialize(serde_json::json!([
        {"id": "cakes", "name": "Cakes", "spec": spec("cakes", "#8e3b46", "cake", 7), "state": "answering"},
        {"id": "bread", "name": "Bread", "spec": spec("bread", "#9c5725", "loaf", 3), "state": "working", "label": "Checking today's stock"},
        {"id": "orders", "name": "Orders", "spec": spec("orders", "#3b2416", "bag", 11), "state": "waiting", "owner": "Wanjiru"},
        {"id": "editor", "name": "Editor", "spec": spec("editor", "#577035", "pen", 5), "state": "paused"},
    ]))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = Arc::new(env());
    let page = {
        let env = env.clone();
        move |Query(q): Query<HashMap<String, String>>| {
            let env = env.clone();
            async move { render(&env, data(&q, "page", "zavora")) }
        }
    };
    let shell = {
        let env = env.clone();
        move |Query(q): Query<HashMap<String, String>>| {
            let env = env.clone();
            async move { render(&env, data(&q, "shell", "zavora")) }
        }
    };
    let site = {
        let env = env.clone();
        move |Path(host): Path<String>, Query(q): Query<HashMap<String, String>>| {
            let env = env.clone();
            async move { render(&env, data(&q, "site", &host)) }
        }
    };
    let error = {
        let env = env.clone();
        move |Path(status): Path<u16>| {
            let env = env.clone();
            async move {
                let ctx = Value::from_serialize(serde_json::json!({
                    "status": status, "who": "wanjiru@example.com", "ref": "r-7f3a21"
                }));
                Html(
                    env.get_template("ui/error.html")
                        .and_then(|t| t.render(ctx))
                        .unwrap_or_else(|e| format!("{e:#}")),
                )
            }
        }
    };
    let app = Router::new()
        .route("/", get(page))
        .route("/shell", get(shell))
        .route("/site/{host}", get(site))
        .route("/error/{status}", get(error))
        .merge(agentrix_ui::assets())
        .merge(agentrix_marks::assets::router());
    let port = std::env::var("PORT").unwrap_or_else(|_| "18700".into());
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    println!("gallery on http://localhost:{port}/");
    axum::serve(listener, app).await?;
    Ok(())
}

fn render(env: &Environment<'static>, ctx: Value) -> Html<String> {
    Html(
        env.get_template("gallery.html")
            .and_then(|t| t.render(ctx))
            .unwrap_or_else(|e| format!("<pre>{e:#}</pre>")),
    )
}
