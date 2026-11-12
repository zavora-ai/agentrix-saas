//! The Cakes desk on Kilele's sample host (ADR-032), with a stand-in model and a fake image
//! gateway: `POST /cakes/talk` keeps only real options and never states a price, falls back to
//! rules without a model; `POST /cakes/preview` labels, caches and caps the idea, and the held
//! request carries the idea the customer saw.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agentrix_samples::ports::{AnswerModel, SampleHost, Visitor, Visitors};
use agentrix_samples::{App, Config};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const HOST: &str = "kilele-bakery-sample.localhost";

/// A customer, as W6 would report them.
struct Customer(&'static str);

#[async_trait::async_trait]
impl Visitors for Customer {
    async fn visitor(
        &self,
        _site: &SampleHost,
        _headers: &HeaderMap,
    ) -> anyhow::Result<Option<Visitor>> {
        Ok(Some(Visitor {
            account: self.0.into(),
            name: "Amani".into(),
            email: "amani.w@example.com".into(),
            email_verified: true,
            phone: Some("+44 7700 900431".into()),
        }))
    }
}

/// The desk's model, scripted: each turn takes the next reply, and every prompt is kept.
#[derive(Default)]
struct Scripted {
    replies: Mutex<VecDeque<Result<String, String>>>,
    prompts: Mutex<Vec<(String, String)>>,
}

impl Scripted {
    fn then(&self, reply: Result<Value, &str>) {
        self.replies.lock().unwrap().push_back(match reply {
            Ok(v) => Ok(v.to_string()),
            Err(e) => Err(e.to_owned()),
        });
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
        anyhow::bail!("the desk asks for whole replies")
    }

    async fn reply(
        &self,
        _site: &str,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> anyhow::Result<String> {
        assert_eq!(schema["required"][0], "say");
        self.prompts
            .lock()
            .unwrap()
            .push((system.to_owned(), user.to_owned()));
        match self.replies.lock().unwrap().pop_front() {
            Some(Ok(text)) => Ok(text),
            Some(Err(e)) => Err(anyhow::anyhow!(e)),
            None => Err(anyhow::anyhow!("no gateway")),
        }
    }
}

/// The fake image gateway: every prompt it was asked to draw.
#[derive(Clone, Default)]
struct Drawn(Arc<Mutex<Vec<String>>>);

fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

async fn fake_draw(
    State(drawn): State<Drawn>,
    axum::Json(body): axum::Json<Value>,
) -> axum::Json<Value> {
    let n = {
        let mut d = drawn.0.lock().unwrap();
        d.push(body["prompt"].as_str().unwrap_or_default().to_owned());
        d.len()
    };
    let mut bytes = b"RIFF\x1a\x00\x00\x00WEBPVP8 cake-idea ".to_vec();
    bytes.extend_from_slice(n.to_string().as_bytes());
    axum::Json(json!({
        "data": [{ "b64_json": b64(&bytes), "media_type": "image/webp" }],
        "size": body["size"], "usage": { "cost": 0.004 }
    }))
}

async fn fake_screen() -> axum::Json<Value> {
    axum::Json(
        json!({ "safe": true, "user_safety": "safe", "categories": [], "model": "content-safety" }),
    )
}

async fn fake_describe() -> axum::Json<Value> {
    axum::Json(json!({
        "alt": "A two-tier cake in forest green buttercream with berries on top",
        "photographic": true, "shows_person": false, "shows_text": false, "check": ""
    }))
}

struct Rig {
    router: Router,
    app: App,
    model: Arc<Scripted>,
    drawn: Drawn,
}

async fn rig(account: &'static str) -> Rig {
    let drawn = Drawn::default();
    let gateway = Router::new()
        .route("/v1/images/generations", post(fake_draw))
        .route("/v1/images/screen", post(fake_screen))
        .route("/v1/images/describe", post(fake_describe))
        .with_state(drawn.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    ctx.platform_host = "localhost".into();
    ctx.models_url = format!("http://{addr}");
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
        include_str!("../../agentrix-node/src/db/migrations/1202_media_ideas.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&ctx.pool).await.unwrap();
    }
    let config = Config {
        data_dir: std::env::temp_dir().join(agentrix_records::new_id("agentrix-samples-desk")),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let model = Arc::new(Scripted::default());
    let app = App::new(config, &ctx)
        .unwrap()
        .with_visitors(Arc::new(Customer(account)))
        .with_model(model.clone());
    let router = agentrix_samples::mounted(app.clone(), agentrix_media::public_router(&ctx))
        .hosts
        .into_iter()
        .next()
        .unwrap()
        .router;
    Rig {
        router,
        app,
        model,
        drawn,
    }
}

impl Rig {
    async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        let req = Request::post(path)
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn get(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::get(path)
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    fn draws(&self) -> usize {
        self.drawn.0.lock().unwrap().len()
    }
}

fn talk(text: &str, spec: Value) -> Value {
    json!({ "text": text, "picked": false, "thread": [], "spec": spec })
}

#[tokio::test]
async fn the_desk_keeps_only_real_options_and_never_states_a_price() {
    let rig = rig("cus-desk-talk").await;

    // Codes that aren't on the price list are dropped; what isn't made is worded by the site.
    rig.model.then(Ok(json!({
        "say": "A forest theme on two tiers, with an ombre fading to cream. Which flavour would she like?",
        "set": { "size": "size.2", "sponge": "sponge.pistachio", "colour": "colour.forest",
                 "finish": "finish.ombre", "deco": "deco.gold-leaf", "message": null,
                 "fulfilment": "drone", "date": "2099-01-01" },
        "occasion": "birthday", "theme": "the forest",
        "not_offered": ["sculpted figures"],
        "pass_to_owner": null, "ask": "sponge"
    })));
    let (status, turn) = rig
        .post(
            "/cakes/talk",
            &talk(
                "My daughter Amina turns 7, she loves the forest. About 25 kids, pistachio sponge, gold leaf and a unicorn figure on top, by drone.",
                json!({ "occasion_asked": true }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(turn["by"], "model", "{turn}");
    assert_eq!(turn["set"]["size"], "size.2");
    assert_eq!(turn["set"]["colour"], "colour.forest");
    assert_eq!(turn["set"]["finish"], "finish.ombre");
    for dropped in ["sponge", "deco", "fulfilment", "date"] {
        assert!(turn["set"][dropped].is_null(), "{dropped} kept: {turn}");
    }
    assert_eq!(turn["occasion"], "Birthday");
    assert_eq!(turn["theme"], "the forest");
    assert_eq!(turn["not_offered"], json!(["Sculpted figures"]));
    assert_eq!(
        turn["say"][0],
        "Kilele doesn't make sculpted figures, so I won't promise that."
    );
    assert_eq!(turn["ask"], "sponge");
    assert_eq!(turn["meta"], "Matched to Kilele's own options");
    assert_eq!(turn["draw"], false, "no picture without a sponge");

    // The prompt carries the options and the calendar, never a price.
    let (system, user) = rig.model.prompts.lock().unwrap()[0].clone();
    assert!(system.contains("sponge.lemon = Lemon sponge"));
    assert!(system.contains("sponge.gf = Gluten-free sponge, 1 and 2 tiers only"));
    assert!(system.contains("Sculpted figures"));
    for price in ["KSh", "4,500", "4500", "3,800", "USD"] {
        assert!(!system.contains(price), "the prompt shows {price}");
    }
    assert!(user.contains("pistachio sponge"));
    assert!(user.contains("Still missing: size, sponge"));

    // A reply that states a price is replaced with the step's fixed question.
    rig.model.then(Ok(json!({
        "say": "Lemon it is. Two tiers come to KSh 4,500. What colour buttercream?",
        "set": { "size": null, "sponge": "sponge.lemon", "colour": null, "finish": null,
                 "deco": null, "message": null, "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "colour"
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk("Lemon please", json!({ "size": "size.2" })),
        )
        .await;
    assert_eq!(turn["set"]["sponge"], "sponge.lemon");
    assert_eq!(turn["say"], json!(["What colour buttercream?"]));
    assert_eq!(turn["draw"], false, "no picture without a colour");

    // Once the size, sponge and colour are known, a new look draws an idea (the desk's
    // picture tool), even with the finish and decoration still open.
    rig.model.then(Ok(json!({
        "say": "Chocolate it is. Which finish would you like?",
        "set": { "size": null, "sponge": "sponge.chocolate", "colour": null, "finish": null,
                 "deco": null, "message": null, "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "finish",
        "draw": false
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk(
                "chocolate",
                json!({ "size": "size.2", "colour": "colour.sky" }),
            ),
        )
        .await;
    assert_eq!(turn["draw"], true, "{turn}");
    // Asked to show it, with nothing new picked, the desk draws too.
    rig.model.then(Ok(json!({
        "say": "Here you go.",
        "set": { "size": null, "sponge": null, "colour": null, "finish": null,
                 "deco": null, "message": null, "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "finish",
        "draw": true
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk(
                "show me what it looks like",
                json!({ "size": "size.2", "sponge": "sponge.chocolate", "colour": "colour.sky" }),
            ),
        )
        .await;
    assert_eq!(turn["draw"], true, "{turn}");

    // Gluten-free on three tiers is dropped with the rule's words; asking an answered step
    // moves on to the next one missing.
    rig.model.then(Ok(json!({
        "say": "Gluten-free sponge, then. How many guests?",
        "set": { "size": null, "sponge": "sponge.gf", "colour": null, "finish": null,
                 "deco": null, "message": "Happy 7th, Amina and all her forest friends", "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "size"
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk(
                "gluten free",
                json!({ "size": "size.3", "occasion_asked": true }),
            ),
        )
        .await;
    assert!(turn["set"]["sponge"].is_null(), "{turn}");
    assert!(turn["set"]["message"].is_null(), "too long for the plaque");
    let said = turn["say"].to_string();
    assert!(
        said.contains("Gluten-free sponge is for 1 and 2 tiers."),
        "{said}"
    );
    assert!(said.contains("the plaque fits 22"), "{said}");
    assert_eq!(turn["ask"], "sponge");

    // An allergy brings the site's own note and an offer to ask Wanjiru, never a claim.
    rig.model.then(Ok(json!({
        "say": "Thanks for telling me. Which sponge would you like?",
        "set": { "size": null, "sponge": null, "colour": null, "finish": null,
                 "deco": null, "message": null, "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "sponge"
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk("My son is allergic to nuts", json!({})),
        )
        .await;
    assert!(
        turn["say"][0]
            .as_str()
            .unwrap()
            .starts_with("From the site: \u{201c}Kilele's kitchen uses wheat flour"),
        "{turn}"
    );
    assert_eq!(turn["pass"]["label"], "Ask Wanjiru");
    assert_eq!(turn["pass"]["question"], "My son is allergic to nuts");

    // A claim about an allergen is replaced too.
    rig.model.then(Ok(json!({
        "say": "The vanilla sponge is nut-free. What colour?",
        "set": { "size": null, "sponge": null, "colour": null, "finish": null,
                 "deco": null, "message": null, "fulfilment": null, "date": null },
        "occasion": null, "theme": null, "not_offered": [], "pass_to_owner": null, "ask": "colour"
    })));
    let (_, turn) = rig
        .post(
            "/cakes/talk",
            &talk(
                "is vanilla ok",
                json!({ "size": "size.1", "sponge": "sponge.vanilla" }),
            ),
        )
        .await;
    assert_eq!(turn["say"], json!(["What colour buttercream?"]));
}

#[tokio::test]
async fn without_a_model_the_page_matches_words_without_the_ai_label() {
    let rig = rig("cus-desk-fallback").await;
    rig.model.then(Err("connection refused"));
    let (status, turn) = rig
        .post("/cakes/talk", &talk("two tiers, lemon", json!({})))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(turn["by"], "rules");
    assert_eq!(turn["say"], json!([]));
    // An answer that isn't the JSON asked for counts as no answer.
    rig.model.then(Ok(json!("Lovely, two tiers it is.")));
    let (_, turn) = rig
        .post("/cakes/talk", &talk("two tiers, lemon", json!({})))
        .await;
    assert_eq!(turn["by"], "rules");
    // The page itself never claims AI before a model answers.
    let (_, page) = rig.get("/cakes/studio").await;
    let page = String::from_utf8_lossy(&page);
    assert!(
        page.contains("<b id=\"deskName\" data-ai=\"Cakes\">Cake studio</b>"),
        "{page}"
    );
    assert!(page.contains("id=\"deskAi\" hidden>AI</span>"));
    assert!(page.contains("The studio matches your words to Kilele's own options."));
}

fn cake(colour: &str, finish: &str, message: &str) -> Value {
    json!({ "size": "size.2", "sponge": "sponge.lemon", "colour": colour, "finish": finish,
            "deco": "deco.berries", "message": message })
}

/// A day the studio offers for collection of two tiers.
async fn a_collection_day(rig: &Rig) -> String {
    let (_, page) = rig.get("/cakes/studio").await;
    let page = String::from_utf8_lossy(&page).into_owned();
    let start = page.find("id=\"studio-data\">").unwrap() + "id=\"studio-data\">".len();
    let end = start + page[start..].find("</script>").unwrap();
    let data: Value = serde_json::from_str(&page[start..end]).unwrap();
    data["days"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["why"]["size.2"]["collection"] == "")
        .and_then(|d| d["date"].as_str())
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn an_idea_is_labelled_cached_capped_and_carried_by_the_request() {
    let rig = rig("cus-desk-idea").await;

    // Nothing is drawn until every option the picture needs is real.
    let (status, body) = rig
        .post(
            "/cakes/preview",
            &json!({ "size": "size.2", "sponge": "sponge.lemon" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["words"],
        "To draw it, pick the buttercream colour first. The finish and decoration can wait."
    );
    assert_eq!(body["missing"], json!(["colour"]));
    let (status, _) = rig
        .post(
            "/cakes/preview",
            &cake("colour.unicorn", "finish.ombre", ""),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rig.draws(), 0);

    // An unchosen finish and decoration are drawn plain, and the answer says so.
    let (status, plain) = rig
        .post(
            "/cakes/preview",
            &json!({ "size": "size.2", "sponge": "sponge.chocolate", "colour": "colour.sky",
                     "message": "Happy birthday Liam" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    assert_eq!(plain["filled"], json!(["finish.smooth", "deco.none"]));
    assert_eq!(
        plain["note"],
        "I've drawn it with a smooth finish and no decoration. Tell me if you'd like another finish, or something on top such as fresh berries."
    );
    assert_eq!(
        plain["design"],
        json!({ "finish": "finish.smooth", "deco": "deco.none" })
    );
    assert_eq!(
        plain["label"],
        "An idea of your cake, made with AI. Wanjiru bakes the real one."
    );
    let drawn = rig.drawn.0.lock().unwrap()[0].clone();
    assert!(
        drawn.contains("smooth sky blue buttercream, with a plain top"),
        "{drawn}"
    );
    // Choosing those same plain options later is the same picture, with nothing to say.
    let (_, same) = rig
        .post(
            "/cakes/preview",
            &json!({ "size": "size.2", "sponge": "sponge.chocolate", "colour": "colour.sky",
                     "finish": "finish.smooth", "deco": "deco.none", "message": "Hi" }),
        )
        .await;
    assert_eq!(same["cached"], true);
    assert!(same["note"].is_null());
    assert_eq!(same["url"], plain["url"]);
    // Only the decoration open: the note names it alone.
    let (_, deco_only) = rig
        .post(
            "/cakes/preview",
            &json!({ "size": "size.2", "sponge": "sponge.chocolate", "colour": "colour.sky",
                     "finish": "finish.smooth", "message": "Hi" }),
        )
        .await;
    assert_eq!(
        deco_only["note"],
        "I've drawn it with no decoration. Tell me if you'd like something on top, such as fresh berries."
    );
    assert_eq!(rig.draws(), 1);

    // Drawn from the options only, labelled, never lettered.
    let (status, idea) = rig
        .post(
            "/cakes/preview",
            &cake("colour.forest", "finish.ombre", "Happy 7th, Amina"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{idea}");
    assert_eq!(
        idea["label"],
        "An idea of your cake, made with AI. Wanjiru bakes the real one."
    );
    assert_eq!(idea["cached"], false);
    assert_eq!(idea["plaque"], "Happy 7th, Amina");
    let url = idea["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/media/ideas/"));
    let prompt = rig.drawn.0.lock().unwrap()[1].clone();
    assert!(
        prompt.starts_with(
            "A two-tier round celebration cake (two stacked round tiers, the top one smaller), covered in forest green buttercream in an ombre"
        ),
        "{prompt}"
    );
    assert!(prompt.contains("blank cream plaque"));
    assert!(!prompt.contains("Amina"), "the message is never drawn");
    assert!(prompt.contains("No text, letters"));

    // The same design again is the same picture, without a model call.
    let (_, again) = rig
        .post(
            "/cakes/preview",
            &cake("colour.forest", "finish.ombre", "Another message"),
        )
        .await;
    assert_eq!(again["cached"], true);
    assert_eq!(again["url"], idea["url"]);
    assert_eq!(rig.draws(), 2);
    let (status, bytes) = rig.get(&url).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(b"RIFF"));

    // A naked finish is drawn without the word the picture policy refuses.
    let (status, _) = rig
        .post("/cakes/preview", &cake("colour.cream", "finish.naked", ""))
        .await;
    assert_eq!(status, StatusCode::OK);
    let naked = rig.drawn.0.lock().unwrap()[2].clone();
    assert!(
        naked.contains("semi-bare finish") && !naked.contains("naked"),
        "{naked}"
    );

    // Five new pictures a day per visitor; designs already drawn still show.
    for (colour, finish) in [
        ("colour.blush", "finish.smooth"),
        ("colour.sky", "finish.drip"),
    ] {
        let (status, _) = rig.post("/cakes/preview", &cake(colour, finish, "")).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(rig.draws(), 5);
    let (status, capped) = rig
        .post("/cakes/preview", &cake("colour.sky", "finish.smooth", ""))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        capped["words"]
            .as_str()
            .unwrap()
            .contains("5 new pictures today")
    );
    assert_eq!(rig.draws(), 5);
    let (status, _) = rig
        .post(
            "/cakes/preview",
            &cake("colour.forest", "finish.ombre", "Hello"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "a drawn design costs nothing");

    // The held request carries the idea the customer saw, and only that one.
    let date = a_collection_day(&rig).await;
    let mut request = cake("colour.forest", "finish.ombre", "Happy 7th, Amina");
    request["date"] = json!(date);
    request["fulfilment"] = json!("collection");
    request["idempotency_key"] = json!("desk-idea-1");
    request["idea"] = idea["digest"].clone();
    let (status, held) = rig.post("/cakes/request", &request).await;
    assert_eq!(status, StatusCode::OK, "{held}");
    let records = rig.app.records().await.unwrap();
    let site = records.site("kilele-bakery-sample").await.unwrap().unwrap();
    let order = records
        .order(&site, held["order_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(order.order.spec().unwrap()["idea"], json!(url));

    // A picture of another design is never attached.
    request["colour"] = json!("colour.sky");
    request["finish"] = json!("finish.drip");
    request["idempotency_key"] = json!("desk-idea-2");
    let (status, held) = rig.post("/cakes/request", &request).await;
    assert_eq!(status, StatusCode::OK, "{held}");
    let order = records
        .order(&site, held["order_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(order.order.spec().unwrap().get("idea").is_none());
}

#[tokio::test]
async fn the_cakes_desk_answers_under_the_rules_the_owner_taught_the_concierge() {
    use agentrix_kernel::taught::{GUARD, RULES_HEADING};
    let rig = rig("cus-desk-rules").await;
    let pool = &rig.app.control;
    agentrix_decisions::migrate(pool).await.unwrap();
    let now = || chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    sqlx::query("INSERT INTO decision_rules (id, site_id, webagent, job, kind, words, from_card, created_by, created_at, receipt) VALUES ('rule_tiers', 'kilele-bakery-sample', 'concierge', 'presentations', 'answer.publish', 'Suggest two tiers for more than 20 guests', 'dec2_x', 'owner', ?, 'r')")
        .bind(now())
        .execute(pool)
        .await
        .unwrap();
    let reply = || {
        Ok(json!({
            "say": "Lovely. How many guests are you expecting?",
            "set": {}, "occasion": "birthday", "theme": null,
            "not_offered": [], "pass_to_owner": null, "ask": "size"
        }))
    };

    rig.model.then(reply());
    let (status, _) = rig
        .post("/cakes/talk", &talk("A birthday cake please", json!({})))
        .await;
    assert_eq!(status, StatusCode::OK);
    let system = rig.model.prompts.lock().unwrap()[0].0.clone();
    assert!(
        system.contains("- «Suggest two tiers for more than 20 guests» (taught "),
        "{system}"
    );
    let at = |needle: &str| {
        system
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in {system}"))
    };
    assert!(at("Never state, estimate or compare a price") < at(RULES_HEADING));
    assert!(at(RULES_HEADING) < at(GUARD), "the guard follows the rule");
    assert!(at(GUARD) < at("Answer with one JSON object only"));

    // Removed: the next turn no longer carries it.
    sqlx::query("UPDATE decision_rules SET removed_at = ? WHERE id = 'rule_tiers'")
        .bind(now())
        .execute(pool)
        .await
        .unwrap();
    rig.model.then(reply());
    rig.post("/cakes/talk", &talk("A birthday cake please", json!({})))
        .await;
    let system = rig.model.prompts.lock().unwrap()[1].0.clone();
    assert!(!system.contains("two tiers for more"), "{system}");
    assert!(!system.contains(RULES_HEADING), "{system}");
}
