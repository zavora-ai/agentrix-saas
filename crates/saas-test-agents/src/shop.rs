//! A test agent finishing a checkout on the site, as a customer would, with no person present:
//! it opens the link the door made, signs in with its `test-agent-<name>@example.com` address
//! (on sample sites the code shows on the page, labelled, and no email goes out), places the
//! order for collection or delivery, starts the payment, confirms PayPal's order with its saved
//! sandbox buyer, and returns to the site, which captures. On the newspaper it subscribes for a
//! Supporter month the same way, on the site's own subscribe page. Sandbox only.

use std::collections::BTreeMap;

use reqwest::StatusCode;
use reqwest::header::{COOKIE, LOCATION, ORIGIN, SET_COOKIE};

use crate::identity::Agent;
use crate::pay::Buyers;

/// How a paid checkout ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paid {
    /// The site's order id.
    pub order_id: String,
    /// PayPal's order id.
    pub paypal_order: String,
}

/// How a subscription ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subscribed {
    /// Paid at PayPal with the saved buyer; the site granted the month.
    Paid {
        /// PayPal's order id.
        paypal_order: String,
    },
    /// The site has no PayPal and granted the month with its sample payment.
    StandIn,
    /// Already a subscriber: the site took nothing.
    Already,
}

/// A browser session on one site: cookies, no redirects followed.
struct Browser<'a> {
    http: &'a reqwest::Client,
    origin: String,
    cookies: BTreeMap<String, String>,
    user_agent: String,
}

struct Answer {
    status: StatusCode,
    location: Option<String>,
    body: String,
}

impl Browser<'_> {
    fn take_cookies(&mut self, response: &reqwest::Response) {
        for value in response.headers().get_all(SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let pair = text.split(';').next().unwrap_or_default();
            if let Some((name, value)) = pair.split_once('=') {
                if value.is_empty() {
                    self.cookies.remove(name.trim());
                } else {
                    self.cookies
                        .insert(name.trim().to_owned(), value.trim().to_owned());
                }
            }
        }
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn absolute(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_owned()
        } else {
            format!("{}{path}", self.origin)
        }
    }

    async fn send(&mut self, request: reqwest::RequestBuilder) -> anyhow::Result<Answer> {
        let response = request
            .header("user-agent", &self.user_agent)
            .header(COOKIE, self.cookie_header())
            .send()
            .await?;
        self.take_cookies(&response);
        let status = response.status();
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await.unwrap_or_default();
        Ok(Answer {
            status,
            location,
            body,
        })
    }

    async fn get(&mut self, path: &str) -> anyhow::Result<Answer> {
        let url = self.absolute(path);
        let request = self.http.get(url);
        self.send(request).await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> anyhow::Result<Answer> {
        let url = self.absolute(path);
        let request = self
            .http
            .post(url)
            .header(ORIGIN, self.origin.clone())
            .form(form);
        self.send(request).await
    }
}

/// The text of the first `.note`/error on a page, for the log.
fn page_words(body: &str) -> String {
    let start = body
        .find("role=\"alert\"")
        .or_else(|| body.find("class=\"note"));
    let text = start.map_or("", |i| &body[i..]);
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
        if out.len() > 160 {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The code a sample site shows a test agent on its code page.
fn shown_code(body: &str) -> Option<String> {
    let i = body.find("id=\"acct-test-code\">")? + "id=\"acct-test-code\">".len();
    let code: String = body[i..].chars().take_while(char::is_ascii_digit).collect();
    (code.len() == 6).then_some(code)
}

/// Signs in (or creates the account the first time), ending with a customer session; `next`
/// is where the site goes after.
async fn sign_in(browser: &mut Browser<'_>, agent: &Agent, next: &str) -> anyhow::Result<()> {
    let email = format!("test-agent-{}@example.com", agent.slug);
    let name = agent.display();
    let mut pending = None;
    for purpose in ["sign-in", "create"] {
        let sent = browser
            .post(
                "/account/code/send",
                &[
                    ("email", &email),
                    ("purpose", purpose),
                    ("name", &name),
                    ("next", next),
                ],
            )
            .await?;
        if let Some(to) = sent.location.filter(|l| l.contains("/account/code?p=")) {
            pending = Some(to);
            break;
        }
    }
    let Some(code_page) = pending else {
        anyhow::bail!("the site didn't send a sign-in code");
    };
    let page = browser.get(&code_page).await?;
    let code = shown_code(&page.body)
        .ok_or_else(|| anyhow::anyhow!("the site didn't show the test agent's code"))?;
    let p = code_page
        .split_once("p=")
        .map(|(_, p)| p.split('&').next().unwrap_or_default())
        .unwrap_or_default();
    let p = url::form_urlencoded::parse(format!("p={p}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    browser
        .cookies
        .insert("agx_passkey_offer".into(), "no".into());
    let checked = browser
        .post("/account/code/check", &[("pending", &p), ("code", &code)])
        .await?;
    if checked.status != StatusCode::SEE_OTHER {
        anyhow::bail!(
            "the sign-in code was refused: {}",
            page_words(&checked.body)
        );
    }
    Ok(())
}

/// Finishes the checkout `link` and pays with the agent's saved buyer. `slot` and `zone` come
/// from the door's `delivery.slots`.
pub async fn pay(
    http: &reqwest::Client,
    buyers: &Buyers,
    agent: &Agent,
    site: &str,
    link: &str,
    slot: Option<&str>,
    zone: Option<&str>,
) -> anyhow::Result<Paid> {
    let vault_id = buyers.vault_id(agent).ok_or_else(|| {
        anyhow::anyhow!(
            "no saved sandbox buyer yet (approve one at {}/paypal/save)",
            agent.origin
        )
    })?;
    let mut browser = Browser {
        http,
        origin: site.trim_end_matches('/').to_owned(),
        cookies: BTreeMap::new(),
        user_agent: agent.user_agent(),
    };
    let claimed = browser.get(link).await?;
    if claimed.status != StatusCode::SEE_OTHER {
        anyhow::bail!("the checkout link didn't open a bag ({})", claimed.status);
    }
    sign_in(&mut browser, agent, "/checkout").await?;

    // Place the order: collection at the bakery, delivery elsewhere.
    let bakery = site.contains("bakery");
    let key = uuid::Uuid::now_v7().simple().to_string();
    let recipient = format!("{}, a test address (no delivery happens)", agent.display());
    let mut form: Vec<(&str, &str)> = vec![("key", &key)];
    if bakery {
        form.push(("how", "collection"));
    } else {
        form.push(("how", "delivery"));
        form.push(("recipient", &recipient));
    }
    if let Some(s) = slot {
        form.push(("slot", s));
    }
    if let Some(z) = zone.filter(|_| !bakery) {
        form.push(("zone", z));
    }
    form.push(("note", "Test order from a test customer agent (sandbox)."));
    let placed = browser.post("/checkout", &form).await?;
    let order_pay = placed
        .location
        .filter(|l| l.contains("/orders/") && l.ends_with("/pay"))
        .ok_or_else(|| anyhow::anyhow!("checkout refused: {}", page_words(&placed.body)))?;
    let order_id = order_pay
        .trim_end_matches("/pay")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();

    // Start the payment: the site sends the customer to PayPal's approval page.
    let started = browser.post(&order_pay, &[]).await?;
    let approve = started
        .location
        .filter(|l| l.contains("paypal.com"))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the site didn't send us to PayPal: {}",
                page_words(&started.body)
            )
        })?;
    let token = approval_token(&approve)?;

    // The saved buyer approves, with no one present; then the site's return URL captures.
    let status = buyers.confirm(&token, &vault_id).await?;
    if status != "APPROVED" && status != "COMPLETED" {
        anyhow::bail!("PayPal didn't approve with the saved buyer ({status})");
    }
    let back = browser
        .get(&format!("/orders/{order_id}/paid?token={token}"))
        .await?;
    let landed = back.location.unwrap_or_default();
    if back.status == StatusCode::SEE_OTHER && !landed.contains("/pay") {
        Ok(Paid {
            order_id,
            paypal_order: token,
        })
    } else {
        anyhow::bail!("the site didn't capture: {}", page_words(&back.body))
    }
}

/// The PayPal order in an approval link's `token`.
fn approval_token(approve: &str) -> anyhow::Result<String> {
    url::Url::parse(approve)?
        .query_pairs()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| anyhow::anyhow!("PayPal's page names no order"))
}

/// Subscribes for one Supporter month on a newspaper's own page (`/subscribe`), paid with the
/// agent's saved sandbox buyer.
pub async fn subscribe(
    http: &reqwest::Client,
    buyers: &Buyers,
    agent: &Agent,
    site: &str,
) -> anyhow::Result<Subscribed> {
    let vault_id = buyers.vault_id(agent).ok_or_else(|| {
        anyhow::anyhow!(
            "no saved sandbox buyer yet (approve one at {}/paypal/save)",
            agent.origin
        )
    })?;
    let mut browser = Browser {
        http,
        origin: site.trim_end_matches('/').to_owned(),
        cookies: BTreeMap::new(),
        user_agent: agent.user_agent(),
    };
    sign_in(&mut browser, agent, "/subscribe").await?;
    let started = browser.post("/subscribe", &[]).await?;
    let to = started.location.unwrap_or_default();
    if to.contains("paypal.com") {
        let token = approval_token(&to)?;
        let status = buyers.confirm(&token, &vault_id).await?;
        if status != "APPROVED" && status != "COMPLETED" {
            anyhow::bail!("PayPal didn't approve with the saved buyer ({status})");
        }
        let back = browser
            .get(&format!("/subscribe/paid?token={token}"))
            .await?;
        if back.status == StatusCode::SEE_OTHER
            && back.location.as_deref().is_some_and(|l| l.contains("paid=1"))
        {
            return Ok(Subscribed::Paid {
                paypal_order: token,
            });
        }
        anyhow::bail!("the site didn't capture: {}", page_words(&back.body));
    }
    if started.status == StatusCode::SEE_OTHER && to.contains("paid=1") {
        return Ok(Subscribed::StandIn);
    }
    if started.status == StatusCode::SEE_OTHER && to.ends_with("/subscribe") {
        return Ok(Subscribed::Already);
    }
    if to.contains("/account/sign-in") {
        anyhow::bail!("the site didn't keep the sign-in");
    }
    anyhow::bail!(
        "the site didn't send us to PayPal: {}",
        page_words(&started.body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shown_code_is_read_from_the_page() {
        let body = r#"<p class="note"><b>Test customer agent.</b> … <b class="mono" id="acct-test-code">042917</b>.</p>"#;
        assert_eq!(shown_code(body).as_deref(), Some("042917"));
        assert_eq!(shown_code("<b id=\"acct-test-code\">12</b>"), None);
    }

    #[test]
    fn the_paypal_order_is_read_from_the_approval_link() {
        assert_eq!(
            approval_token("https://www.sandbox.paypal.com/checkoutnow?token=5O190127TN364715T")
                .unwrap(),
            "5O190127TN364715T"
        );
        assert!(approval_token("https://www.sandbox.paypal.com/checkoutnow").is_err());
    }
}
