//! A scripted, seeded model: deterministic tests, CI, and judge-mode demo takes at zero model
//! cost (pattern: steward-core's `scripted.rs`).
//!
//! Each call to the model plays the next [`Turn`]: tool calls, final text, one of several texts
//! picked by the seed, or a turn computed from the request (the tool results so far), which is
//! how a script reacts to what the books said ("propose the plan memory suggested"). When the
//! script runs out the model says "Done." Call ids are `call-<seed>-<n>`, so the same seed gives
//! the same transcript, ledger keys and approval fingerprints on every take.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use async_trait::async_trait;
use serde_json::Value;

/// A turn computed from the request.
pub type TurnFn = Arc<dyn Fn(&LlmRequest) -> Turn + Send + Sync>;

/// One model turn.
#[derive(Clone)]
pub enum Turn {
    /// The model calls these tools (name, args).
    Calls(Vec<(String, Value)>),
    /// The model answers with text and ends its turn.
    Text(String),
    /// The model answers with one of these texts, picked by the seed.
    OneOf(Vec<String>),
    /// The turn depends on the request: the tool results so far.
    Dynamic(TurnFn),
}

impl std::fmt::Debug for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calls(calls) => f.debug_tuple("Calls").field(calls).finish(),
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::OneOf(texts) => f.debug_tuple("OneOf").field(texts).finish(),
            Self::Dynamic(_) => f.write_str("Dynamic(..)"),
        }
    }
}

struct State {
    turns: VecDeque<Turn>,
    fallback: Option<TurnFn>,
    calls: u64,
    rng: u64,
    requests: Vec<LlmRequest>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("turns", &self.turns)
            .field("reacting", &self.fallback.is_some())
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

/// A model that plays back a script.
#[derive(Debug, Clone)]
pub struct ScriptedLlm {
    name: String,
    seed: u64,
    state: Arc<Mutex<State>>,
}

impl ScriptedLlm {
    /// An empty script with seed 0.
    pub fn new(name: impl Into<String>) -> Self {
        Self::seeded(name, 0)
    }

    /// An empty script with `seed`.
    pub fn seeded(name: impl Into<String>, seed: u64) -> Self {
        Self {
            name: name.into(),
            seed,
            state: Arc::new(Mutex::new(State {
                turns: VecDeque::new(),
                fallback: None,
                calls: 0,
                rng: seed ^ 0x9E37_79B9_7F4A_7C15,
                requests: Vec::new(),
            })),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// When the script runs out, every turn is computed by `f` instead of "Done.": a model
    /// that reacts to what it is given (judge mode).
    pub fn reacting(self, f: impl Fn(&LlmRequest) -> Turn + Send + Sync + 'static) -> Self {
        self.state().fallback = Some(Arc::new(f));
        self
    }

    /// Appends a turn with one tool call.
    pub fn call(self, tool: impl Into<String>, args: Value) -> Self {
        self.push(Turn::Calls(vec![(tool.into(), args)]))
    }

    /// Appends a final text turn.
    pub fn text(self, text: impl Into<String>) -> Self {
        self.push(Turn::Text(text.into()))
    }

    /// Appends a turn computed from the request.
    pub fn then(self, f: impl Fn(&LlmRequest) -> Turn + Send + Sync + 'static) -> Self {
        self.push(Turn::Dynamic(Arc::new(f)))
    }

    /// Appends any turn.
    pub fn push(self, turn: Turn) -> Self {
        self.state().turns.push_back(turn);
        self
    }

    /// Appends turns while the model is in use (a later demo step).
    pub fn extend(&self, turns: impl IntoIterator<Item = Turn>) {
        self.state().turns.extend(turns);
    }

    /// Every request the model was sent.
    pub fn requests(&self) -> Vec<LlmRequest> {
        self.state().requests.clone()
    }

    /// How many turns are left.
    pub fn remaining(&self) -> usize {
        self.state().turns.len()
    }

    fn next_random(state: &mut State) -> u64 {
        // xorshift64*: enough to pick a phrasing, the same every take.
        let mut x = state.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        state.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// The newest response from tool `name` in `req`, if any.
pub fn last_tool_result(req: &LlmRequest, name: &str) -> Option<Value> {
    req.contents.iter().rev().find_map(|c| {
        c.parts.iter().rev().find_map(|p| match p {
            Part::FunctionResponse {
                function_response, ..
            } if function_response.name == name => Some(function_response.response.clone()),
            _ => None,
        })
    })
}

/// The text of the newest user message in `req`.
pub fn last_user_text(req: &LlmRequest) -> String {
    req.contents
        .iter()
        .rev()
        .find(|c| c.role == "user" && c.parts.iter().any(|p| p.text().is_some()))
        .map(|c| {
            c.parts
                .iter()
                .filter_map(|p| p.text())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[async_trait]
impl Llm for ScriptedLlm {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate_content(
        &self,
        req: LlmRequest,
        _stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let content = {
            let mut state = self.state();
            state.requests.push(req.clone());
            let fallback = state.fallback.clone();
            let mut turn = state.turns.pop_front().unwrap_or_else(|| match fallback {
                Some(f) => Turn::Dynamic(f),
                None => Turn::Text("Done.".into()),
            });
            // A dynamic turn may itself compute a dynamic turn; resolve to a concrete one.
            for _ in 0..4 {
                match turn {
                    Turn::Dynamic(f) => turn = f(&req),
                    other => {
                        turn = other;
                        break;
                    }
                }
            }
            match turn {
                Turn::Text(text) => Content::new("model").with_text(text),
                Turn::OneOf(texts) if !texts.is_empty() => {
                    let pick = (Self::next_random(&mut state) % texts.len() as u64) as usize;
                    Content::new("model").with_text(texts[pick].clone())
                }
                Turn::OneOf(_) | Turn::Dynamic(_) => Content::new("model").with_text("Done."),
                Turn::Calls(calls) => {
                    let mut content = Content::new("model");
                    for (name, args) in calls {
                        state.calls += 1;
                        content.parts.push(Part::FunctionCall {
                            name,
                            args,
                            id: Some(format!("call-{}-{}", self.seed, state.calls)),
                            thought_signature: None,
                        });
                    }
                    content
                }
            }
        };
        let response = LlmResponse::new(content);
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;

    async fn play(llm: &ScriptedLlm, req: LlmRequest) -> Content {
        let mut s = llm.generate_content(req, false).await.unwrap();
        s.next().await.unwrap().unwrap().content.unwrap()
    }

    #[tokio::test]
    async fn the_same_seed_plays_the_same_take() {
        let script = |seed| {
            ScriptedLlm::seeded("m", seed)
                .call("ar_ageing", json!({}))
                .push(Turn::OneOf(vec!["a".into(), "b".into(), "c".into()]))
        };
        let (a, b) = (script(7), script(7));
        let req = LlmRequest::new("m", vec![]);
        assert_eq!(play(&a, req.clone()).await, play(&b, req.clone()).await);
        assert_eq!(play(&a, req.clone()).await, play(&b, req.clone()).await);
        assert_eq!(play(&a, req).await.parts[0].text(), Some("Done."));
    }

    #[tokio::test]
    async fn a_dynamic_turn_reads_the_tool_results() {
        let llm = ScriptedLlm::new("m").then(|req| match last_tool_result(req, "ar_ageing") {
            Some(v) => Turn::Text(format!("owed {}", v["total"])),
            None => Turn::Text("nothing yet".into()),
        });
        let mut tool = Content::new("user");
        tool.parts.push(Part::FunctionResponse {
            function_response: adk_core::FunctionResponseData {
                name: "ar_ageing".into(),
                response: json!({ "total": 42 }),
                inline_data: vec![],
                file_data: vec![],
            },
            id: None,
            annotations: None,
        });
        let content = play(&llm, LlmRequest::new("m", vec![tool])).await;
        assert_eq!(content.parts[0].text(), Some("owed 42"));
    }
}
