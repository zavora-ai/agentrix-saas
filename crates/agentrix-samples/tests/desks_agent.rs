//! The sample desks as WebAgents, with a stand-in model: Ask's desks write from the site's rules
//! and pages, pick the desk by what the question is about and read the conversation so far;
//! Classy's search is read and answered by the model with rules behind it; instant answers
//! never call a model; and whatever no model wrote never says AI.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agentrix_samples::ports::AnswerModel;
use agentrix_samples::{App, Config};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const KILELE: &str = "kilele-bakery-sample.localhost";
const CLASSY: &str = "classy-perfumes-sample.localhost";
const TANA: &str = "tana-valley-times-sample.localhost";

/// The desks' model, scripted: each call takes the next reply (none left: the gateway is down),
/// and every prompt is kept.
#[derive(Default)]
struct Scripted {
    replies: Mutex<VecDeque<Result<Value, String>>>,
    prompts: Mutex<Vec<(String, String, Value)>>,
}

impl Scripted {
    fn then(&self, reply: Value) {
        self.replies.lock().unwrap().push_back(Ok(reply));
    }
    fn calls(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }
    /// The (system, user) prompts of call `n` (0-based).
    fn prompt(&self, n: usize) -> (String, String) {
        let p = self.prompts.lock().unwrap();
        (p[n].0.clone(), p[n].1.clone())
    }
}

#[async_trait::async_trait]
impl AnswerModel for Scripted {
    async fn answer(
        &self,
        _site: &str,
        _system: &str,
        _user: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        anyhow::bail!("the desks ask for whole replies")
    }

    async fn reply(
        &self,
        _site: &str,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> anyhow::Result<String> {
        self.prompts
            .lock()
            .unwrap()
            .push((system.to_owned(), user.to_owned(), schema.clone()));
        match self.replies.lock().unwrap().pop_front() {
            Some(Ok(v)) => Ok(v.to_string()),
            Some(Err(e)) => Err(anyhow::anyhow!(e)),
            None => Err(anyhow::anyhow!("no gateway")),
        }
    }
}

struct Rig {
    router: Router,
    /// The owner apps (platform host).
    platform: Router,
    model: Arc<Scripted>,
    /// Node's `control.db`.
    control: sqlx::SqlitePool,
}

async fn rig() -> Rig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    ctx.platform_host = "localhost".into();
    let config = Config {
        data_dir: std::env::temp_dir()
            .join(format!("agentrix-samples-desks-{}", rand::random::<u64>())),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let model = Arc::new(Scripted::default());
    let app = App::new(config, &ctx).unwrap().with_model(model.clone());
    let mounted = agentrix_samples::mounted(app, Router::new());
    let router = mounted.hosts.into_iter().next().unwrap().router;
    Rig {
        router,
        platform: mounted.platform,
        model,
        control: ctx.pool.clone(),
    }
}

/// Keeps `words` as the owner's rule for `webagent` on Kilele, as declining one of its cards
/// with "Make this a rule" does; `removed` as removed from its page since.
async fn rule(pool: &sqlx::SqlitePool, id: &str, webagent: &str, words: &str, removed: bool) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    sqlx::query("INSERT INTO decision_rules (id, site_id, webagent, job, kind, words, from_card, created_by, created_at, receipt, removed_at) VALUES (?, 'kilele-bakery-sample', ?, 'presentations', 'answer.publish', ?, 'dec2_x', 'owner', ?, 'r', ?)")
        .bind(id)
        .bind(webagent)
        .bind(words)
        .bind(&now)
        .bind(removed.then_some(&now))
        .execute(pool)
        .await
        .unwrap();
}

impl Rig {
    async fn get(&self, host: &str, path: &str) -> (StatusCode, String) {
        let req = Request::get(path)
            .header(header::HOST, host)
            .body(Body::empty())
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn post(&self, host: &str, path: &str, body: &Value) -> (StatusCode, String) {
        let req = Request::post(path)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Signs in to Kilele's owner apps with the sample owner's details; the session cookie.
    async fn wanjiru(&self) -> String {
        let req = Request::post("/samples/kilele-bakery-sample/sign-in")
            .header(header::HOST, "localhost")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                "email=wanjiru%40kilele-bakery.example&password=kilele-sample-owner",
            ))
            .unwrap();
        let res = self.platform.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        res.headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("agx_owner_kilele_bakery_sample="))
            .and_then(|v| v.split(';').next())
            .unwrap()
            .to_owned()
    }

    /// An owner page, as text (minijinja's escapes undone).
    async fn owner_get(&self, path: &str, cookie: &str) -> String {
        let req = Request::get(path)
            .header(header::HOST, "localhost")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        let res = self.platform.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        unescape(&String::from_utf8_lossy(&bytes))
    }

    /// Posts an owner form; it answers 303 back to the app.
    async fn owner_post(&self, path: &str, form: &str, cookie: &str) {
        let req = Request::post(path)
            .header(header::HOST, "localhost")
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .unwrap();
        let res = self.platform.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "{path}");
        let to = res.headers()[header::LOCATION].to_str().unwrap();
        assert!(to.starts_with("/samples/kilele-bakery-sample/"), "{to}");
    }

    /// `POST /ask/feedback`: its JSON reply.
    async fn mark(&self, host: &str, body: Value) -> Value {
        let (status, text) = self.post(host, "/ask/feedback", &body).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    /// `POST /ask`: the `answer` event's data.
    async fn ask(&self, host: &str, body: Value) -> Value {
        let (status, text) = self.post(host, "/ask", &body).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let mut event = "";
        for line in text.lines() {
            if let Some(e) = line.strip_prefix("event:") {
                event = e.trim();
            } else if let Some(d) = line.strip_prefix("data:")
                && event == "answer"
            {
                return serde_json::from_str(d.trim()).unwrap();
            }
        }
        panic!("no answer event in {text}");
    }
}

/// Text as a reader sees it: minijinja escapes `/`, quotes and `&` in values.
fn unescape(html: &str) -> String {
    html.replace("&#x2f;", "/")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn said(a: &Value) -> String {
    a["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::test]
async fn a_desk_writes_from_the_site_with_the_conversation_and_hands_over_by_intent() {
    let rig = rig().await;
    rig.model.then(json!({
        "desk": "bread",
        "covered": true,
        "answer": "Without dairy or egg, the **country sourdough** is the one to pick [1]. Everything else uses butter or milk [2].",
        "action": "bread"
    }));
    let a = rig
        .ask(
            KILELE,
            json!({
                "q": "Anything without dairy?",
                "desk": "cakes",
                "history": [
                    { "who": "visitor", "text": "How much for 30 people?" },
                    { "who": "Cakes", "text": "For about 30 people, 2 tiers fits." }
                ]
            }),
        )
        .await;
    assert_eq!(a["basis"], "ai", "{a}");
    // The model chose the desk by what the question is about, not by the page it came from.
    assert_eq!(a["desk"]["id"], "bread");
    assert!(
        a["handover"]
            .as_str()
            .unwrap()
            .starts_with("Cakes handed you to Bread, with the conversation so far"),
        "{a}"
    );
    assert!(said(&a).contains("**country sourdough**"), "{a}");
    // Only cited sources are kept, numbered as cited.
    let sources = a["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2, "{a}");
    assert_eq!(sources[0]["n"], 1);
    assert!(
        a["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["href"] == "/bread"),
        "{a}"
    );
    // The conversation so far, the site's rules and its pages went with the question.
    let (system, user) = rig.model.prompt(0);
    assert!(system.contains("- bread: Bread."), "{system}");
    assert!(system.contains("Never state a price"), "{system}");
    assert!(
        user.contains("Visitor: How much for 30 people?")
            && user.contains("Cakes: For about 30 people"),
        "{user}"
    );
    assert!(user.contains("worked out by rules"), "{user}");
    assert!(
        user.ends_with("Question: Anything without dairy?"),
        "{user}"
    );
}

#[tokio::test]
async fn a_number_no_source_holds_is_dropped_for_the_rules_answer() {
    let rig = rig().await;
    rig.model.then(json!({
        "desk": "cakes",
        "covered": true,
        "answer": "Two tiers cost **KSh 9,999** [1].",
        "action": "studio"
    }));
    let a = rig
        .ask(KILELE, json!({ "q": "how much is a two tier cake?" }))
        .await;
    assert_eq!(rig.model.calls(), 1);
    assert_eq!(a["basis"], "rules", "{a}");
    assert!(!said(&a).contains("9,999"), "{a}");
    assert!(said(&a).contains("KSh"), "{a}");

    // A checking failure doesn't rest the model: the next question still reaches it.
    // The model judges that the site doesn't cover it: the gap answer, by AI, with the owner.
    rig.model.then(json!({
        "desk": "front", "covered": false, "answer": "", "action": "ask_owner"
    }));
    let a = rig
        .ask(
            KILELE,
            json!({ "q": "Do you run baking classes for kids on weekdays?" }),
        )
        .await;
    assert_eq!(rig.model.calls(), 2);
    assert_eq!(a["basis"], "ai", "{a}");
    assert!(said(&a).contains("doesn't say"), "{a}");
    assert!(
        a["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["kind"] == "pass"),
        "{a}"
    );
}

#[tokio::test]
async fn without_the_gateway_rules_answer_with_no_ai_label_and_the_model_rests() {
    let rig = rig().await;
    let (_, home) = rig.get(CLASSY, "/").await;
    assert!(
        home.contains(">AI answers<"),
        "the chip shows while the model answers"
    );

    // No reply queued: the gateway is down.
    let a = rig.ask(CLASSY, json!({ "q": "Can I return it?" })).await;
    assert_eq!(rig.model.calls(), 1);
    assert_eq!(a["basis"], "rules", "{a}");
    assert_eq!(a["meta"], "From the returns policy");
    assert_eq!(a["desk"]["id"], "orders");

    // Resting: the next answers come from rules at once, without trying the model.
    let a = rig
        .ask(CLASSY, json!({ "q": "Is gift wrapping free?" }))
        .await;
    assert_eq!(rig.model.calls(), 1);
    assert_eq!(a["basis"], "rules", "{a}");
    assert_eq!(a["desk"]["id"], "gifts");

    // ...and the pages stop promising AI answers meanwhile.
    let (_, home) = rig.get(CLASSY, "/").await;
    assert!(!home.contains(">AI answers<"), "no AI chip while resting");
}

#[tokio::test]
async fn instant_answers_come_from_records_without_a_model() {
    let rig = rig().await;
    // "today" doesn't make it a stock question: whole words decide.
    let a = rig.ask(KILELE, json!({ "q": "Are you open today?" })).await;
    assert_eq!(a["basis"], "instant", "{a}");
    assert_eq!(a["desk"]["id"], "visit");
    assert!(!said(&a).contains("sourdough"), "{a}");

    let a = rig
        .ask(KILELE, json!({ "q": "Can you hold 2 loaves for me?" }))
        .await;
    assert_eq!(a["basis"], "instant", "{a}");
    assert!(
        a["actions"][0]["kind"] == "hold" || said(&a).contains("can't hold"),
        "{a}"
    );

    let a = rig
        .ask(CLASSY, json!({ "q": "Is Citrus Dawn in stock?" }))
        .await;
    assert_eq!(a["basis"], "instant", "{a}");

    let a = rig
        .ask(TANA, json!({ "q": "Who has been reporting it?" }))
        .await;
    assert_eq!(a["basis"], "instant", "{a}");
    assert_eq!(rig.model.calls(), 0);

    // The peek tags only these starters "Instant".
    let (_, page) = rig.get(KILELE, "/").await;
    let instant = |q: &str| {
        let at = page.find(&format!("data-q=\"{q}\"")).expect(q);
        let button = &page[at..];
        button[..button.find("</button>").unwrap()].contains("<small>Instant</small>")
    };
    assert!(instant("Are you open on Sunday?"));
    assert!(!instant("Where do I park?"));
}

#[tokio::test]
async fn an_order_in_the_conversation_gets_a_card_with_its_total_and_a_pay_button() {
    let rig = rig().await;
    let a = rig
        .ask(KILELE, json!({ "q": "I'd like two sourdough loaves please" }))
        .await;
    let pay = a["actions"]
        .as_array()
        .and_then(|x| x.iter().find(|x| x["kind"] == "pay"))
        .unwrap_or_else(|| panic!("no pay card: {a}"));
    assert_eq!(pay["card"]["lines"][0]["sku"], "sourdough", "{a}");
    assert_eq!(pay["card"]["lines"][0]["qty"], 2, "{a}");
    assert_eq!(pay["card"]["total"], "KSh 900", "{a}");
    assert_eq!(pay["href"], "/checkout", "{a}");
    assert!(said(&a).contains("nothing is charged until you do"), "{a}");
    // A question about an allergen is still the allergens answer, not an order.
    let a = rig
        .ask(KILELE, json!({ "q": "Can I get a gluten-free loaf?" }))
        .await;
    assert!(
        a["actions"].as_array().is_none_or(|x| x.iter().all(|x| x["kind"] != "pay")),
        "{a}"
    );
}

#[tokio::test]
async fn catch_me_up_is_written_by_the_newsroom_from_published_stories() {
    let rig = rig().await;
    rig.model.then(json!({
        "desk": "newsroom",
        "covered": true,
        "answer": "This week the paper kept to the rail extension and the markets [1].",
        "action": "digest"
    }));
    let a = rig
        .ask(
            TANA,
            json!({ "q": "Catch me up on this week", "desk": "newsroom" }),
        )
        .await;
    assert_eq!(a["basis"], "ai", "{a}");
    assert_eq!(a["desk"]["id"], "newsroom");
    let (system, user) = rig.model.prompt(0);
    assert!(system.contains("Never give an opinion"), "{system}");
    assert!(user.contains("worked out by rules"), "{user}");

    // "front" fits no desk: a reader already talking to a desk stays with it.
    rig.model.then(json!({
        "desk": "front",
        "covered": true,
        "answer": "The paper's subscription is listed on the subscribe page [1].",
        "action": "subscribe"
    }));
    let a = rig
        .ask(
            TANA,
            json!({ "q": "and is there a yearly plan?", "desk": "subscriptions",
                    "history": [{ "who": "visitor", "text": "How much is a subscription?" }] }),
        )
        .await;
    assert_eq!(a["desk"]["id"], "subscriptions", "{a}");
    assert!(a["handover"].is_null(), "{a}");

    // Without the model, the published stories are listed by rules, with no AI label.
    let a = rig
        .ask(
            TANA,
            json!({ "q": "Catch me up on this week", "desk": "newsroom" }),
        )
        .await;
    assert_eq!(a["basis"], "rules", "{a}");
    let (_, home) = rig.get(TANA, "/").await;
    assert!(
        !home.contains("AI · from published stories only"),
        "resting"
    );
    assert!(home.contains(">from published stories only<"));
}

#[tokio::test]
async fn classy_search_is_read_and_answered_by_the_desks_model_ranked_by_rules() {
    let rig = rig().await;
    let q = "a gift under KSh 5,000 for someone who loves vanilla";
    let enc = "a%20gift%20under%20KSh%205%2C000%20for%20someone%20who%20loves%20vanilla";

    // The page comes at once, by rules, and asks for the desks' answer.
    let (status, page) = rig.get(CLASSY, &format!("/search?q={enc}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("data-agent=\"&#x2f;search&#x2f;agent?q=a%20gift"),
        "agent link"
    );
    assert!(page.contains("AI calls</span><b>0</b>"));
    assert_eq!(rig.model.calls(), 0);
    let card = &page[page.find("id=\"answer\"").expect("a rules answer card")..];
    let card = &card[..card.find("</div></div>").unwrap()];
    assert!(!card.contains(">AI<"), "rules answers say no AI: {card}");

    // The agent body: read by the model, ranked by rules, written by the model.
    rig.model.then(json!({
        "budget_ksh": 5000, "family": null, "vanilla": true, "gift": true, "size": null,
        "strength": null, "product": null, "town": null, "delivery": false, "needs": []
    }));
    rig.model.then(json!({
        "desk": "gifts",
        "covered": true,
        "answer": "The best fit is the **Vanilla Gift Set** at **KSh 4,500** [1], already wrapped with a card. Amber Noir is over budget [2].",
        "action": "wrapping"
    }));
    let (status, body) = rig.get(CLASSY, &format!("/search/agent?q={enc}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.model.calls(), 2);
    assert!(
        body.trim_start().starts_with("<div class=\"s-body"),
        "a fragment"
    );
    assert!(!body.contains("data-agent"), "{body}");
    let card = &body[body.find("id=\"answer\"").expect("answer card")..];
    assert!(card.contains("<span class=\"wa-ai\">AI</span>"), "{card}");
    assert!(card.contains("Vanilla Gift Set"));
    assert!(body.contains("AI calls</span><b>2</b>"));
    assert!(body.contains("Read by AI from your words"));
    // Prices and fit badges come from records and rules, not from the model.
    assert!(body.contains("✓ Fits budget"), "{body}");
    // Each chip drops its part and keeps the rest of the reading, with no model call.
    assert!(body.contains("r=b%3A500000%3Bg"), "{body}");
    let (system, user) = rig.model.prompt(0);
    assert!(
        system.contains("vanilla-gift-set: Vanilla Gift Set"),
        "{system}"
    );
    assert_eq!(user, format!("Search: {q}"));
    let (_, user) = rig.model.prompt(1);
    assert!(
        user.contains("shortlist place 1 (Best fit). Vanilla Gift Set"),
        "{user}"
    );

    let (_, chip) = rig.get(CLASSY, &format!("/search?q={enc}&r=v%3Bg")).await;
    assert_eq!(rig.model.calls(), 2, "a chip link never calls the model");
    assert!(!chip.contains("data-agent"));
    assert!(chip.contains("Vanilla Gift Set"));

    // The gateway goes down: the same body by rules, and no AI label.
    let (_, body) = rig.get(CLASSY, &format!("/search/agent?q={enc}")).await;
    assert_eq!(rig.model.calls(), 3);
    let card = &body[body.find("id=\"answer\"").expect("answer card")..];
    let card = &card[..card.find("</div></div>").unwrap()];
    assert!(!card.contains(">AI<"), "{card}");
    assert!(body.contains("AI calls</span><b>0</b>"));
    assert!(!body.contains("Read by AI"));
    assert!(body.contains("Vanilla Gift Set"));
}

#[tokio::test]
async fn a_product_name_goes_straight_to_the_product_without_a_model() {
    let rig = rig().await;
    let (_, page) = rig.get(CLASSY, "/search?q=rain%20oud").await;
    assert!(!page.contains("data-agent"));
    assert!(page.contains("Exact match"));
    assert_eq!(rig.model.calls(), 0);
}

#[tokio::test]
async fn the_agent_door_says_ai_only_when_a_model_wrote_the_answer() {
    let rig = rig().await;
    let call = |q: &str| {
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
               "params": {"name": "ask_site", "arguments": {"question": q}}})
    };
    rig.model.then(json!({
        "desk": "visit",
        "covered": true,
        "answer": "Parking is at the back, off **Woodvale Grove** [1].",
        "action": "visit"
    }));
    let (_, body) = rig.post(KILELE, "/mcp", &call("Where do I park?")).await;
    let v: Value = serde_json::from_str(&body).unwrap();
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("Written by the Visit desk, an AI WebAgent"),
        "{v}"
    );
    assert_eq!(v["result"]["structuredContent"]["ai"], true, "{v}");

    let (_, body) = rig
        .post(KILELE, "/mcp", &call("Are you open on Sunday?"))
        .await;
    let v: Value = serde_json::from_str(&body).unwrap();
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(!text.contains("AI WebAgent"), "{v}");
    assert!(text.contains("without a model"), "{v}");
    assert_eq!(v["result"]["structuredContent"]["ai"], false, "{v}");
}

#[tokio::test]
async fn the_visitors_own_number_may_be_repeated() {
    let rig = rig().await;
    rig.model.then(json!({
        "desk": "front",
        "covered": true,
        "answer": "Under **KSh 8,000**, look at the gift sets [1].",
        "action": "none"
    }));
    let a = rig
        .ask(
            CLASSY,
            json!({ "q": "Something under KSh 8,000 for my mum? She hates sweet scents." }),
        )
        .await;
    assert_eq!(rig.model.calls(), 1);
    assert_eq!(a["basis"], "ai", "{a}");
    assert!(said(&a).contains("KSh 8,000"), "{a}");
}

/// The Model Gateway refuses sites control.db doesn't list, so on a fresh node every desk fell
/// back to rules until each sample had a row there.
#[tokio::test]
async fn each_sample_is_a_site_the_platform_knows() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ctx = agentrix_host::testing::context().await.unwrap();
    // The first release's `sites` columns that the samples fill in.
    sqlx::query(
        "CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', \
         old_origin TEXT NOT NULL, utc_offset_minutes INTEGER NOT NULL DEFAULT 0, owner_account_id TEXT, \
         fixture INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();
    // A row already there, with an owner, stays as it is.
    sqlx::query(
        "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, owner_account_id, created_at) \
         VALUES ('kilele-bakery-sample', 'kilele-bakery-sample.localhost', 'Kilele Bakery', \
         'https://kilele-bakery.example', 180, 'acct-1', '2026-11-08T08:33:52Z')",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();
    let config = Config {
        data_dir: std::env::temp_dir()
            .join(format!("agentrix-samples-sites-{}", rand::random::<u64>())),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let app = App::new(config, &ctx).unwrap();
    app.records().await.unwrap();

    let rows: Vec<(String, String, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT site_id, domain, name, utc_offset_minutes, fixture, owner_account_id FROM sites ORDER BY site_id",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        ids,
        [
            "classy-perfumes-sample",
            "kilele-bakery-sample",
            "tana-valley-times-sample"
        ]
    );
    for (id, domain, name, offset, fixture, _) in &rows {
        assert_eq!(domain, &format!("{id}.localhost"));
        assert!(!name.is_empty(), "{id} has its business name");
        assert_eq!(*offset, 180, "{id} keeps Nairobi time");
        assert_eq!(*fixture, 0, "{id} is a sample, not a fixture");
    }
    assert_eq!(
        rows[1].5.as_deref(),
        Some("acct-1"),
        "Kilele's owner is kept"
    );
}

#[tokio::test]
async fn the_buttons_follow_what_the_model_recommends() {
    let rig = rig().await;
    let q = "My mum hates sweet scents. What would you suggest under KSh 8,000 as a birthday gift?";
    // A first turn shows the numbered sources the desks' model reads for this question.
    rig.model
        .then(json!({ "desk": "gifts", "covered": false, "answer": "", "action": "none" }));
    rig.ask(CLASSY, json!({ "q": q })).await;
    let (_, user) = rig.model.prompt(0);
    let products: Vec<(String, String)> = user
        .lines()
        .filter_map(|l| {
            let (n, rest) = l.strip_prefix('[')?.split_once("] ")?;
            n.parse::<usize>().ok()?;
            Some((n.to_owned(), rest.split(':').next()?.trim().to_owned()))
        })
        .filter(|(_, label)| label != "Gift wrapping")
        .collect();
    assert!(products.len() >= 2, "{user}");
    // The rules ranked one first; the model recommends another and advises against the first.
    let ((top_n, top), (pick_n, pick)) = (&products[0], &products[1]);
    rig.model.then(json!({
        "desk": "gifts",
        "covered": true,
        "answer": format!(
            "For someone who doesn't like sweet scents, the **{pick}** is the one I'd pick [{pick_n}]. I'd avoid the **{top}** [{top_n}] for her."
        ),
        "action": "none"
    }));
    let a = rig.ask(CLASSY, json!({ "q": q })).await;
    assert_eq!(a["basis"], "ai", "{a}");
    let actions = a["actions"].as_array().unwrap();
    // The primary link is the product the answer recommends...
    let page = a["sources"][0]["href"]
        .as_str()
        .unwrap()
        .split('#')
        .next()
        .unwrap();
    assert_eq!(a["sources"][0]["label"], pick.as_str(), "{a}");
    assert_eq!(actions[0]["href"], page, "{a}");
    assert!(
        actions[0]["label"]
            .as_str()
            .unwrap()
            .contains(pick.as_str()),
        "{a}"
    );
    assert_eq!(
        actions.iter().filter(|x| x["primary"] == true).count(),
        1,
        "{a}"
    );
    // ...and no button pushes the one it advises against, or compares it.
    for x in actions {
        let label = x["label"].as_str().unwrap();
        assert!(
            !label.contains(top.as_str()) && !label.contains("Compare"),
            "{a}"
        );
    }
}

#[tokio::test]
async fn a_question_with_more_in_it_goes_past_the_instant_answer() {
    let rig = rig().await;
    // The plain price question is the capability's whole: instant, no model.
    let a = rig
        .ask(KILELE, json!({ "q": "How much is a cake for 15 people?" }))
        .await;
    assert_eq!(a["basis"], "instant", "{a}");
    assert_eq!(rig.model.calls(), 0);

    // With a second ask, the model writes it, with the instant answer among its facts.
    rig.model.then(json!({
        "desk": "cakes",
        "covered": true,
        "answer": "For 15 people, a cake from the studio fits [1].",
        "action": "none"
    }));
    let q = "I'm planning a small office party for 15 people on Friday. What would you recommend, and can you deliver?";
    let a = rig.ask(KILELE, json!({ "q": q })).await;
    assert_eq!(rig.model.calls(), 1, "{a}");
    assert_eq!(a["basis"], "ai", "{a}");
    let (_, user) = rig.model.prompt(0);
    assert!(user.contains("worked out by rules"), "{user}");
    assert!(user.ends_with(&format!("Question: {q}")), "{user}");
    // The second ask's page is among the facts, and no page is given twice.
    let facts = &user[..user.rfind("Question:").unwrap()];
    assert!(facts.to_lowercase().contains("deliver"), "{user}");
    let labels: Vec<&str> = facts
        .lines()
        .filter(|l| l.starts_with('['))
        .map(|l| l.split_once("] ").map_or(l, |(_, r)| r))
        .map(|l| l.split(':').next().unwrap_or(l))
        .collect();
    for (i, l) in labels.iter().enumerate() {
        assert!(!labels[..i].contains(l), "{l} twice: {user}");
    }
}

#[tokio::test]
async fn every_starter_tagged_instant_is_answered_from_records_without_a_model() {
    let rig = rig().await;
    for host in [KILELE, CLASSY, TANA] {
        let (_, page) = rig.get(host, "/").await;
        let page = unescape(&page);
        let mut tagged = Vec::new();
        for chunk in page.split("<button class=\"starter\"").skip(1) {
            let button = &chunk[..chunk.find("</button>").unwrap()];
            if !button.contains("<small>Instant</small>") {
                continue;
            }
            let q = button
                .split("data-q=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap()
                .to_owned();
            let a = rig.ask(host, json!({ "q": q })).await;
            assert_eq!(a["basis"], "instant", "{host}: {q}: {a}");
            tagged.push(q);
        }
        assert!(!tagged.is_empty(), "{host} tags some starters Instant");
    }
    assert_eq!(rig.model.calls(), 0, "no Instant starter waited on a model");
}

#[tokio::test]
async fn a_taught_answer_reaches_the_desks_and_a_wrong_answer_reaches_the_owner() {
    let rig = rig().await;
    let wanjiru = rig.wanjiru().await;

    // The seed's open question q-s4, asked again: the desks don't have the owner's answer yet.
    let first = rig
        .ask(KILELE, json!({ "q": "Do you deliver to Lavington?" }))
        .await;
    assert_ne!(first["basis"], "owner", "{first}");
    let id = first["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("a-"), "{first}");

    // "Was this right?" No, then a note: one mark, recorded for Wanjiru.
    let r = rig
        .mark(KILELE, json!({ "answer": id, "right": false }))
        .await;
    assert_eq!(
        r["words"],
        "Thanks. Wanjiru sees this and can teach the desks a better answer."
    );
    let r = rig
        .mark(
            KILELE,
            json!({ "answer": id, "right": false, "note": "I live in Lavington." }),
        )
        .await;
    assert_eq!(r["words"], "Thanks. Wanjiru sees your note.");
    // An answer this site never gave isn't recorded.
    let (status, _) = rig
        .post(
            KILELE,
            "/ask/feedback",
            &json!({ "answer": "nope", "right": true }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let page = rig
        .owner_get("/samples/kilele-bakery-sample/questions", &wanjiru)
        .await;
    assert!(
        page.contains("visitors said Yes 0 times and No 1 time"),
        "{page}"
    );
    assert!(
        page.contains("<td>Visit</td><td>0</td><td>1</td>"),
        "{page}"
    );
    assert!(
        page.contains("Their note: &ldquo;I live in Lavington.&rdquo;"),
        "{page}"
    );
    assert!(
        page.contains("The desk said: The site lists Westlands and Kilimani only, so"),
        "the answer's words, without citation numbers: {page}"
    );
    assert!(page.contains("Teach this answer"), "{page}");
    let today = rig
        .owner_get("/samples/kilele-bakery-sample/", &wanjiru)
        .await;
    assert!(
        today.contains("Visitors marked 1 answer wrong in the last 7 days."),
        "{today}"
    );

    // Wanjiru answers the passed question in Questions.
    rig.owner_post(
        "/samples/kilele-bakery-sample/questions/q-s4",
        "answer=Not+yet.+We+deliver+to+Westlands+and+Kilimani+only.",
        &wanjiru,
    )
    .await;

    // The next visitor who asks gets her answer, cited from the owner, without a model.
    let calls = rig.model.calls();
    let a = rig
        .ask(KILELE, json!({ "q": "do you deliver to lavington" }))
        .await;
    assert_eq!(rig.model.calls(), calls);
    assert_eq!(a["basis"], "owner", "{a}");
    assert_eq!(a["desk"]["id"], "visit", "{a}");
    assert_eq!(
        said(&a),
        "Not yet. We deliver to Westlands and Kilimani only. [1]"
    );
    let label = a["sources"][0]["label"].as_str().unwrap();
    assert!(label.starts_with("From the owner, "), "{a}");
    assert!(
        a["meta"]
            .as_str()
            .unwrap()
            .starts_with("Answered by Wanjiru (a person), "),
        "{a}"
    );
    // Another place is another question.
    let a = rig
        .ask(KILELE, json!({ "q": "Do you deliver to Kilimani?" }))
        .await;
    assert_ne!(a["basis"], "owner", "{a}");
    // The mark shows the answer as taught.
    let page = rig
        .owner_get("/samples/kilele-bakery-sample/questions", &wanjiru)
        .await;
    assert!(page.contains("You taught an answer on"), "{page}");

    // A wrong answer nobody passed on can be taught from the mark itself.
    let q = "Do you bake on public holidays?";
    let b = rig.ask(KILELE, json!({ "q": q })).await;
    let bid = b["id"].as_str().unwrap().to_owned();
    rig.mark(KILELE, json!({ "answer": bid, "right": false }))
        .await;
    rig.owner_post(
        &format!("/samples/kilele-bakery-sample/feedback/{bid}"),
        "answer=Yes.+On+public+holidays+we+keep+Sunday+hours.",
        &wanjiru,
    )
    .await;
    let b = rig.ask(KILELE, json!({ "q": q })).await;
    assert_eq!(b["basis"], "owner", "{b}");
    assert_eq!(
        said(&b),
        "Yes. On public holidays we keep Sunday hours. [1]"
    );
    // A Yes is counted too.
    let id = b["id"].as_str().unwrap().to_owned();
    let r = rig
        .mark(KILELE, json!({ "answer": id, "right": true }))
        .await;
    assert_eq!(r["words"], "Thanks. Wanjiru sees this.");
    let today = rig
        .owner_get("/samples/kilele-bakery-sample/", &wanjiru)
        .await;
    assert!(
        today.contains("Visitors marked 1 answer right and 2 wrong in the last 7 days."),
        "{today}"
    );
}

#[tokio::test]
async fn the_desks_answer_under_the_rules_the_owner_taught_the_concierge_until_one_is_removed() {
    use agentrix_kernel::taught::{GUARD, RULES_HEADING};
    let rig = rig().await;
    agentrix_decisions::migrate(&rig.control).await.unwrap();
    rule(
        &rig.control,
        "rule_stall",
        "concierge",
        "Say we also sell at the Saturday market stall",
        false,
    )
    .await;
    rule(
        &rig.control,
        "rule_old",
        "concierge",
        "Say the shop opens at 6",
        true,
    )
    .await;
    rule(
        &rig.control,
        "rule_cash",
        "cashier",
        "Never refund in cash",
        false,
    )
    .await;
    let answer = || {
        json!({
            "desk": "bread",
            "covered": true,
            "answer": "Without dairy or egg, the **country sourdough** is the one to pick [1]. Everything else uses butter or milk [2].",
            "action": "bread"
        })
    };
    let ask = json!({ "q": "Anything without dairy?", "desk": "cakes" });

    rig.model.then(answer());
    assert_eq!(rig.ask(KILELE, ask.clone()).await["basis"], "ai");
    let (system, _) = rig.model.prompt(0);
    assert!(
        system.contains("- «Say we also sell at the Saturday market stall» (taught "),
        "{system}"
    );
    let at = |needle: &str| {
        system
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in {system}"))
    };
    assert!(
        at("Never state a price") < at(RULES_HEADING),
        "the desks' own rules come first"
    );
    assert!(
        system.ends_with(GUARD),
        "the guard closes the prompt: {system}"
    );
    assert!(!system.contains("opens at 6"), "a removed rule stays out");
    assert!(
        !system.contains("in cash"),
        "another WebAgent's rule stays out"
    );

    // Removed on the Concierge's page: the next turn no longer carries it.
    sqlx::query("UPDATE decision_rules SET removed_at = ? WHERE id = 'rule_stall'")
        .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
        .execute(&rig.control)
        .await
        .unwrap();
    rig.model.then(answer());
    rig.ask(KILELE, ask).await;
    let (system, _) = rig.model.prompt(1);
    assert!(!system.contains("Saturday market"), "{system}");
    assert!(!system.contains(RULES_HEADING), "{system}");
}
