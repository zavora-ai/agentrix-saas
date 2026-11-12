//! Messages to customers and owners about orders and the digest, through W9's catalogue and
//! outbox (`agentrix_messages::orders`). Every message is recorded first and shows in the owner's
//! Messages app; until email is switched on it stays "Not sent". A message that fails to record
//! is logged and never stops the order.

use agentrix_messages::orders::{self, Line, OrderEvent, OrderFacts, SiteFacts};
use agentrix_records::orders::OrderView;
use agentrix_records::{Site, money, time};

use crate::App;

/// The site as W9's messages name it.
pub fn site_facts(site: &Site, host: &str) -> SiteFacts {
    let accent = site.fact("/front_color");
    SiteFacts {
        id: site.site.clone(),
        name: site.name.clone(),
        host: host.to_owned(),
        sample: site.is_sample(),
        owner_first: site.fact("/owner/first"),
        accent: (!accent.is_empty()).then_some(accent),
    }
}

/// An order as W9's messages name it, with links on the site's own host.
pub fn order_facts(site: &Site, origin: &str, view: &OrderView) -> OrderFacts {
    let o = &view.order;
    let cur = &o.currency;
    let off = site.utc_offset_min;
    let delivery_line = |l: &agentrix_records::orders::OrderLine| {
        l.rule_code
            .as_deref()
            .is_some_and(|c| c.starts_with("delivery."))
    };
    let paid = view
        .payments
        .iter()
        .any(|p| matches!(p.state.as_str(), "captured" | "sample"));
    OrderFacts {
        order_id: o.order_id.clone(),
        customer_name: o.contact_name.clone(),
        kind: o.kind.clone(),
        lines: view
            .lines
            .iter()
            .filter(|l| !delivery_line(l))
            .map(|l| Line {
                name: l.name.clone(),
                qty: u32::try_from(l.qty).unwrap_or(1),
                amount: money::fmt(cur, l.amount_minor),
            })
            .collect(),
        delivery: (o.delivery_minor > 0).then(|| money::fmt(cur, o.delivery_minor)),
        total: money::fmt(cur, o.total_minor),
        usd: (o.usd_rate_minor > 0).then(|| money::usd(o.usd_cents())),
        rate: (o.usd_rate_minor > 0).then(|| money::rate_line(o.usd_rate_minor)),
        fulfilment: o.fulfilment.clone(),
        slot: view
            .slot
            .as_ref()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| time::fmt_day(o.created_at, off)),
        recipient: o.recipient.clone(),
        note: o.note.clone(),
        order_url: format!("{origin}/orders/{}", o.order_id),
        pay_url: matches!(o.state.as_str(), "approved" | "awaiting_payment")
            .then(|| format!("{origin}/orders/{}/pay", o.order_id)),
        receipt_url: paid.then(|| format!("{origin}/orders/{}/receipt", o.order_id)),
    }
}

impl App {
    /// Where a site's pages are, for links in messages sent from outside a site request (the
    /// owner's apps): the site's canonical host.
    pub fn site_origin(&self, site: &Site) -> (String, String) {
        (site.host.clone(), self.config.origin_for(&site.host))
    }

    /// Tells the customer about an order event, if the order has an email.
    pub async fn tell_customer(
        &self,
        site: &Site,
        host: &str,
        origin: &str,
        view: &OrderView,
        event: OrderEvent,
    ) {
        let Some(email) = view
            .order
            .contact_email
            .as_deref()
            .filter(|e| !e.trim().is_empty())
        else {
            return;
        };
        let out = orders::customer(
            &site_facts(site, host),
            &order_facts(site, origin, view),
            &event,
            email,
        );
        if let Err(e) = self.messages.send(out).await {
            tracing::warn!(error = %e, site = site.site, order = view.order.order_id, "order message not recorded");
        }
    }

    /// Tells the owner a new order or request is in. Sample sites have no owner account in
    /// control.db, so this is recorded only where one exists; a miss is logged.
    pub async fn tell_owner_new(&self, site: &Site, host: &str, origin: &str, view: &OrderView) {
        let (id, json, why, key) = orders::owner_new_order(
            &site_facts(site, host),
            &order_facts(site, origin, view),
            &self.config.platform_origin(),
        );
        if let Err(e) = self
            .messages
            .send_to_site_owner(&site.site, &id, json, &why, Some(key))
            .await
        {
            tracing::debug!(error = %e, site = site.site, "no owner to notify for this sample");
        }
    }

    /// Asks a reader to confirm their digest subscription (double opt-in).
    pub async fn tell_digest_confirm(
        &self,
        site: &Site,
        host: &str,
        email: &str,
        topics: &str,
        confirm_url: &str,
    ) {
        let out = orders::digest_confirm(&site_facts(site, host), email, topics, confirm_url);
        if let Err(e) = self.messages.send(out).await {
            tracing::warn!(error = %e, site = site.site, "digest confirmation not recorded");
        }
    }

    /// Tells a reader their digest has stopped and their topics are deleted.
    pub async fn tell_digest_stopped(
        &self,
        site: &Site,
        host: &str,
        email: &str,
        resubscribe_url: &str,
    ) {
        let out = orders::digest_stopped(&site_facts(site, host), email, Some(resubscribe_url));
        if let Err(e) = self.messages.send(out).await {
            tracing::warn!(error = %e, site = site.site, "digest stop not recorded");
        }
    }
}

/// "Thu 19 Nov, 20:00", 48 hours on: how long a request or payment request stays open.
pub fn two_days_on(site: &Site, now: i64) -> String {
    time::fmt_day_clock(now + 48 * 3600, site.utc_offset_min)
}

/// The step words a customer reads for an order's new state, or `None` for states that have
/// their own message (approved, declined, lapsed) or none.
pub fn status_label(state: &str, fulfilment: &str) -> Option<String> {
    Some(
        match state {
            "baking" => "Baking",
            "ready" if fulfilment == "collection" => "Ready to collect",
            "ready" => "Ready",
            "out" => "Out for delivery",
            "delivered" => "Delivered",
            "collected" => "Collected",
            "cancelled" => "Cancelled",
            _ => return None,
        }
        .to_owned(),
    )
}

/// The owner's usual reply time in words: "2 hours", "3 hours", "a day".
pub fn answer_within(site: &Site) -> String {
    let wait = site.fact("/owner/wait");
    let wait = wait.trim().trim_start_matches('~').trim();
    match wait.strip_suffix(" h") {
        Some(n) => format!("{n} hours"),
        None => wait.to_owned(),
    }
}

impl App {
    /// A custom request is held: the customer hears it waits for the owner and that nothing
    /// was charged; the owner hears a request is in.
    pub async fn tell_held(&self, site: &Site, host: &str, origin: &str, view: &OrderView) {
        let event = OrderEvent::RequestHeld {
            answer_within: answer_within(site),
            expires: two_days_on(site, view.order.created_at),
        };
        self.tell_customer(site, host, origin, view, event).await;
        self.tell_owner_new(site, host, origin, view).await;
    }

    /// An order is paid: the customer gets the confirmation with the receipt link; for a
    /// catalogue order the owner hears a new order is in (a custom one was announced when it
    /// was requested).
    pub async fn tell_paid(&self, ctx: &crate::web::Ctx, view: &OrderView) {
        let (site, host, origin) = (&ctx.site, &ctx.host.host, &ctx.host.origin);
        self.tell_customer(site, host, origin, view, OrderEvent::Received)
            .await;
        if view.order.kind == "catalogue" {
            self.tell_owner_new(site, host, origin, view).await;
        }
    }

    /// [`App::tell_held`] for an order number placed from a site request.
    pub async fn tell_held_by_id(&self, ctx: &crate::web::Ctx, order_id: &str) {
        let Ok(records) = self.records().await else {
            return;
        };
        if let Ok(Some(view)) = records.order(&ctx.site, order_id).await {
            self.tell_held(&ctx.site, &ctx.host.host, &ctx.host.origin, &view)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use agentrix_records::orders::{Contact, Placement};
    use agentrix_records::pricing::CakeChoice;

    use crate::{App, Config};

    #[tokio::test]
    async fn a_held_cake_request_is_recorded_for_the_customer() {
        let mut ctx = agentrix_host::testing::context().await.unwrap();
        ctx.platform_host = "localhost".into();
        agentrix_messages::migrate(&ctx.pool).await.unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config = Config {
            data_dir: std::env::temp_dir()
                .join(format!("agentrix-samples-notify-{}", std::process::id())),
            platform_host: "localhost".into(),
            scheme: "http".into(),
            port: None,
            bundles_dir: Some(root.join("fixtures/samples")),
            templates_dir: None,
            reseed: true,
        };
        let app = App::new(config, &ctx).unwrap();
        let records = app.records().await.unwrap();
        let site = records.site("kilele-bakery-sample").await.unwrap().unwrap();
        let now = agentrix_records::time::now();
        let day = agentrix_records::time::local(now + 5 * 86_400, site.utc_offset_min)
            .format("%Y-%m-%d")
            .to_string();
        let choice = CakeChoice {
            size: Some("size.1".into()),
            sponge: Some("sponge.vanilla".into()),
            colour: Some("colour.cream".into()),
            finish: Some("finish.smooth".into()),
            deco: Some("deco.none".into()),
            message: String::new(),
            date: Some(day),
            note: String::new(),
            idea: None,
        };
        let quote = records.quote_cake(&site, &choice, now).await.unwrap();
        let contact = Contact {
            name: "Test".into(),
            email: Some("test.buyer@example.com".into()),
            ..Default::default()
        };
        let how = Placement {
            channel: "site".into(),
            fulfilment: "collection".into(),
            ..Default::default()
        };
        let view = records
            .request_custom(&site, &choice, &quote, &contact, &how, now)
            .await
            .unwrap();
        app.tell_held(
            &site,
            &site.host,
            "http://kilele-bakery-sample.localhost",
            &view,
        )
        .await;
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM message_outbox WHERE message = 'customer.request_held'",
        )
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "the held request is in the outbox");
        // A retry records nothing twice.
        app.tell_held(
            &site,
            &site.host,
            "http://kilele-bakery-sample.localhost",
            &view,
        )
        .await;
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM message_outbox WHERE message = 'customer.request_held'",
        )
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
    }
}
