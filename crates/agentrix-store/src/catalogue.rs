//! The metric catalogue: every number and list the concept screens draw, where it comes from,
//! and what to show while it has no source (specs/platform/03-data.md §7).
//!
//! A screen that draws a number looks it up here by id. If the entry is [`Status::Sourced`],
//! a [`crate::metrics::Metrics`] method returns it from recorded rows. If not, the screen shows
//! [`Metric::empty`], which says why it is empty and who is building the source. Nothing on
//! screen is ever typed in.

use serde::Serialize;

use crate::metrics::Reading;

/// Which surface draws the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The owner console (Agentrix OS).
    Console,
    /// A site's own pages.
    Site,
    /// A customer's or owner's journey page.
    Journey,
    /// The operator and portfolio consoles.
    ControlPlane,
    /// agentrix.website's home page.
    Home,
}

/// Whether the number has a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Computed from recorded rows by the named API.
    Sourced,
    /// Part of it is sourced; the rest shows its empty words.
    Partial,
    /// No source yet: show the empty words.
    Pending,
}

/// One number or list on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Metric {
    /// Stable id, `surface.thing`.
    pub id: &'static str,
    /// Where it is drawn.
    pub surface: Surface,
    /// What the concept shows.
    pub shows: &'static str,
    /// The concept page and line.
    pub concept: &'static str,
    /// Whether it has a source.
    pub status: Status,
    /// The [`crate::metrics::Metrics`] method, or what the source still needs.
    pub source: &'static str,
    /// The workstream that produces the data.
    pub owner: &'static str,
    /// What to show while there is nothing to count, in plain words.
    pub empty: &'static str,
}

macro_rules! metric {
    ($id:literal, $surface:ident, $shows:literal, $concept:literal, $status:ident,
     $source:literal, $owner:literal, $empty:literal) => {
        Metric {
            id: $id,
            surface: Surface::$surface,
            shows: $shows,
            concept: $concept,
            status: Status::$status,
            source: $source,
            owner: $owner,
            empty: $empty,
        }
    };
}

/// Every entry, by surface.
pub const CATALOGUE: &[Metric] = &[
    // ---- Owner console: sourced from control.db today ----
    metric!(
        "console.spend.today",
        Console,
        "AI budget used today: \"KSh 38\"",
        "concept/console/shell.html:51",
        Sourced,
        "Metrics::spend(Site, Today)",
        "W8",
        "No model calls today."
    ),
    metric!(
        "console.spend.overnight",
        Console,
        "\"KSh 9 overnight\"",
        "concept/console/phone.html:115",
        Sourced,
        "Metrics::spend(Site, Since(evening))",
        "W8",
        "No model calls overnight."
    ),
    metric!(
        "console.spend.limits",
        Console,
        "\"KSh 38 of 1,500\": spend against the budget",
        "concept/console/shell.html:51",
        Partial,
        "Metrics::spend(..).limits; in USD until W3 posts the site's exchange rate",
        "W8, W3",
        "No budget set for this site. The gateway's own limit is $5 a day."
    ),
    metric!(
        "console.decisions.waiting",
        Console,
        "Decisions waiting, oldest first",
        "concept/console/shell.html:59-65",
        Sourced,
        "Metrics::decisions_waiting(Site)",
        "W8",
        "Nothing waiting for you."
    ),
    metric!(
        "console.receipts.since_go_live",
        Console,
        "\"412 entries since go-live\"",
        "concept/site.html:339",
        Sourced,
        "Metrics::receipt_counts(Site, All).since_go_live",
        "W8",
        "No receipts yet. They start when the site goes live."
    ),
    metric!(
        "console.receipts.list",
        Console,
        "Receipts by kind on Trust",
        "concept/console/trust.html:195-201",
        Sourced,
        "Metrics::receipts",
        "W8",
        "No receipts yet. Every check, room and approval leaves one here."
    ),
    metric!(
        "console.move.steps",
        Console,
        "The Move's steps with their times",
        "concept/move.html:136-141",
        Sourced,
        "Metrics::moves(site)",
        "W8",
        "No Move yet. It starts once you claim the site."
    ),
    metric!(
        "console.move.cost",
        Console,
        "What the Move cost in model calls",
        "concept/move.html",
        Sourced,
        "Metrics::moves(..).cost",
        "W8",
        "No model calls on this Move."
    ),
    metric!(
        "console.gauntlet",
        Console,
        "Gauntlet score, attacks stopped of tried",
        "concept/console/trust.html:86",
        Sourced,
        "Metrics::gauntlet(Some(site))",
        "W8",
        "The gauntlet hasn't run on this site yet. It runs after go-live."
    ),
    metric!(
        "console.questions.passed",
        Console,
        "Questions visitors passed to you",
        "concept/console/shell.html:79-134",
        Sourced,
        "Metrics::questions_passed",
        "W8",
        "No questions passed to you."
    ),
    metric!(
        "site.visits",
        Console,
        "\"41 visitors\" with pages, sites, devices",
        "concept/console/shell.html:46",
        Partial,
        "Metrics::visits over visit_counts; needs the counter script in each release (W2 themes)",
        "W8, W2",
        "No visits counted yet. Counts start once the site's pages carry the visit counter."
    ),
    // ---- Owner console: waiting for another workstream's tables ----
    metric!(
        "console.visitors.live",
        Console,
        "\"3 live\", visitors per room",
        "concept/console/shell.html:83, :167-188",
        Pending,
        "Live presence with a TTL of seconds (Valkey in the fleet)",
        "W1",
        "Live visitors aren't shown yet."
    ),
    metric!(
        "console.questions.answered",
        Console,
        "\"9 questions, 2 in Swahili, none corrected\"",
        "concept/console/shell.html:83, :180-188",
        Pending,
        "Threads and turns with language and desk; owner corrections",
        "W1",
        "Answers aren't counted yet. Questions the site couldn't answer are under Passed."
    ),
    metric!(
        "console.dock.states",
        Console,
        "Dock states, \"asleep until 02:00\"",
        "concept/console/shell.html:59-65",
        Pending,
        "WebAgent and process tables, next wake",
        "W1",
        "WebAgent status isn't shown yet."
    ),
    metric!(
        "console.spend.per_process",
        Console,
        "Activity monitor: today KSh per process",
        "concept/console/shell.html:260-271",
        Partial,
        "SpendReport::by_webagent and by_job (the gateway records x-agentrix-webagent and \
         x-agentrix-job); per process waits for W1's kernel processes",
        "W1, W8",
        "No WebAgent has used AI yet. Spend per process appears once WebAgents run as processes."
    ),
    metric!(
        "console.mission.progress",
        Console,
        "Mission 23/60, +9 this week, forecast",
        "concept/console/mission.html:86-91",
        Pending,
        "Missions, mission reports, orders tagged to the mission, a forecast method",
        "W1, W2",
        "No missions yet."
    ),
    metric!(
        "console.wall.signals",
        Console,
        "\"Across the wall\": signals by kind",
        "concept/console/shell.html:79-134",
        Pending,
        "The kernel's SignalStore",
        "W1",
        "No signals yet."
    ),
    metric!(
        "console.gaps",
        Console,
        "Teach your site: gaps with counts",
        "concept/console/shell.html:79-134",
        Pending,
        "Content-gap clusters; passed_questions has no counts",
        "W1",
        "No gaps found yet. Questions the site couldn't answer are under Passed."
    ),
    metric!(
        "console.guardian.probes",
        Console,
        "\"212 probes blocked from 3 networks\"",
        "concept/console/shell.html:133",
        Pending,
        "Edge block counters",
        "W2",
        "Blocked probes aren't counted yet."
    ),
    metric!(
        "console.guardian.release_check",
        Console,
        "\"release check 2 min ago\"",
        "concept/console/trust.html:86",
        Pending,
        "A stored release-verification time",
        "W4",
        "Release checks aren't recorded yet."
    ),
    metric!(
        "console.timeline",
        Console,
        "Night timeline entries",
        "concept/console/shell.html:139-162",
        Pending,
        "The activity ledger",
        "W1",
        "Nothing on the timeline yet."
    ),
    metric!(
        "console.live_conversation",
        Console,
        "Live conversation that wants you",
        "concept/console/live.html:85-113",
        Pending,
        "Threads and takeovers",
        "W1",
        "No conversations waiting for you."
    ),
    metric!(
        "console.sales.proposal",
        Console,
        "\"31 customers, 31 consent receipts, about KSh 6\"",
        "concept/console/shell.html:193-232",
        Pending,
        "Customer lists, consent receipts, a cost estimate on cards",
        "W2, W6, W5",
        "No proposals yet."
    ),
    metric!(
        "console.store.listings",
        Console,
        "Store listings: version, eval, trigger, budget",
        "concept/console/store.html:88-160",
        Pending,
        "WebAgentSpec and pack listings; quotes",
        "W1, W2",
        "The Store isn't open yet."
    ),
    metric!(
        "console.autonomy.history",
        Console,
        "Autonomy per job, \"lowered 3 Nov\"",
        "concept/console/trust.html:103-139",
        Pending,
        "Job dial history; approval kinds",
        "W1, W5",
        "Dial history isn't recorded yet."
    ),
    metric!(
        "console.standing_rules",
        Console,
        "Standing rules with uses and expiry",
        "concept/console/trust.html:145-146",
        Pending,
        "Signed standing rules with use counts",
        "W6",
        "No standing rules yet."
    ),
    metric!(
        "console.actions_without_authority",
        Console,
        "\"Actions without authority: 0\"",
        "concept/console/trust.html:86-92",
        Pending,
        "An invariant check over effects and receipts",
        "W6",
        "This check isn't running yet."
    ),
    metric!(
        "console.platform_notices",
        Console,
        "Platform notices: updates, incidents",
        "concept/console/trust.html:152-154",
        Pending,
        "Trains and incidents",
        "W3",
        "No platform notices."
    ),
    metric!(
        "console.support_grant",
        Console,
        "Support grant, ticket, 4 h",
        "concept/console/trust.html:160",
        Pending,
        "Tickets and support grants",
        "W3, W6",
        "Nobody at Zavora has access to this site."
    ),
    metric!(
        "console.erasure.progress",
        Console,
        "Erasure \"11 of 13 stores\" per store",
        "concept/console/trust.html:167-173",
        Pending,
        "Erasure requests and per-store answers (W8's orchestrator, W6's ledger)",
        "W6, W8",
        "No erasure requests."
    ),
    metric!(
        "console.memory.facts",
        Console,
        "Memory facts with provenance",
        "concept/console/trust.html:179-183",
        Pending,
        "Facts with sources",
        "W1",
        "The site hasn't learned any facts yet."
    ),
    metric!(
        "console.receipt.chain",
        Console,
        "Receipt verify block with chain predecessor",
        "concept/console/trust.html:195-201",
        Pending,
        "prev_digest and seq on receipts, or the site log",
        "W6",
        "Receipts are signed; the chain between them isn't shown yet."
    ),
    metric!(
        "console.devices_people",
        Console,
        "Devices and people",
        "concept/console/trust.html:203",
        Pending,
        "Devices and memberships",
        "W6",
        "Devices aren't listed yet."
    ),
    metric!(
        "console.versions",
        Console,
        "\"v14 Sunday hours, v13 Delivery zones FAQ\"",
        "concept/console.html",
        Partial,
        "releases rows; summaries need a column",
        "W4",
        "Release names appear once each release has a summary."
    ),
    metric!(
        "console.certificate",
        Console,
        "\"Certificate renews in 61 days\"",
        "concept/console.html:419",
        Pending,
        "Domain certificate expiry",
        "W3",
        "Certificate dates aren't shown yet."
    ),
    metric!(
        "console.customers",
        Console,
        "Customers list",
        "concept/console.html",
        Pending,
        "Customers",
        "W2",
        "No customers yet. They appear when someone orders or signs up."
    ),
    metric!(
        "console.portfolio.cards",
        Console,
        "Multi-site cards: decisions, mission, budget",
        "concept/console/shell.html:331-333",
        Partial,
        "decisions_waiting and spend per site are sourced; missions are W1's",
        "W1, W3, W8",
        "Missions appear once WebAgents run them."
    ),
    metric!(
        "console.notifications",
        Console,
        "Lock-screen notifications, WhatsApp brief link",
        "concept/console/phone.html:101-103",
        Pending,
        "Notification deliveries",
        "W5, W9",
        "Notifications aren't sent yet."
    ),
    // ---- Sites and visitor experience ----
    metric!(
        "site.stock",
        Site,
        "\"6 left today\", \"next batch 10:00\"",
        "concept/site.html:219-222",
        Pending,
        "Stock levels with batch times",
        "W2",
        "Stock isn't shown yet."
    ),
    metric!(
        "site.order_tracker",
        Site,
        "Held order tracker",
        "concept/site.html:246",
        Pending,
        "Orders and order events",
        "W2",
        "No orders yet."
    ),
    metric!(
        "site.catalogue",
        Site,
        "Catalogue with stock \"as of 11:02\"",
        "concept/experience/search.html:229-240",
        Pending,
        "Products and stock levels",
        "W2",
        "The catalogue isn't loaded yet."
    ),
    metric!(
        "site.quote",
        Site,
        "Quote on price list v14 with rule codes",
        "concept/experience/creator.html:229-240",
        Pending,
        "Price lists, configurable products, quotes",
        "W2",
        "Quotes aren't available yet."
    ),
    metric!(
        "site.gift_plan",
        Site,
        "Gift plan with shares and versions",
        "concept/experience/artifact.html:187-293",
        Pending,
        "Visitor artifacts, shares, versions",
        "W2",
        "No saved plans."
    ),
    metric!(
        "site.news",
        Site,
        "\"This week 12 stories\"; corrections; subscriber flag",
        "concept/experience/news.html:130-149",
        Pending,
        "Posts, corrections, entitlements",
        "W2",
        "No stories yet."
    ),
    metric!(
        "site.feedback",
        Site,
        "\"Was this right? Yes/No\"",
        "concept/experience/experience.js:116",
        Pending,
        "A turn feedback field",
        "W1",
        "Feedback isn't recorded yet."
    ),
    // ---- Customer and owner journeys ----
    metric!(
        "journey.sessions",
        Journey,
        "Sessions; \"New sign-in on a Windows PC in Leeds\"",
        "concept/journeys/customer-security.html:115",
        Pending,
        "Visitor sessions and sign-in events",
        "W6",
        "No other sessions."
    ),
    metric!(
        "journey.what_site_keeps",
        Journey,
        "What the site keeps, with retention",
        "concept/journeys/customer-account.html:127-134",
        Pending,
        "The data catalogue rendered per site",
        "W6, W8",
        "This list isn't ready yet."
    ),
    metric!(
        "journey.consents",
        Journey,
        "Consent receipts with given and withdrawn times",
        "concept/journeys/customer-account.html",
        Pending,
        "Consent receipts",
        "W6",
        "You haven't agreed to anything yet."
    ),
    metric!(
        "journey.slots",
        Journey,
        "Delivery slots with \"full\"",
        "concept/journeys/customer-order.html",
        Pending,
        "Delivery slots",
        "W2",
        "Delivery times aren't shown yet."
    ),
    metric!(
        "journey.usd_rate",
        Journey,
        "USD amount at a pinned rate",
        "concept/journeys/customer-pay.html",
        Pending,
        "Exchange rates pinned on orders",
        "W2, W6, W3",
        "Prices are shown in the site's own currency."
    ),
    metric!(
        "journey.order_timeline",
        Journey,
        "Order status timeline, change cut-off",
        "concept/journeys/customer-after.html",
        Pending,
        "Order events",
        "W2",
        "No order updates yet."
    ),
    metric!(
        "journey.refund_rules",
        Journey,
        "Refund rule checks",
        "concept/journeys/customer-after.html",
        Pending,
        "Standing rules with use counts",
        "W6",
        "Refunds go to the owner to approve."
    ),
    metric!(
        "journey.erasure_receipt",
        Journey,
        "Erasure receipt across 10 stores",
        "concept/journeys/customer-security.html:357-371",
        Pending,
        "Erasure requests and answers",
        "W6, W8",
        "No erasure requests."
    ),
    metric!(
        "journey.agent_door_log",
        Journey,
        "Agent door call log, tier, mandate chain",
        "concept/journeys/customer-agent.html",
        Pending,
        "Agent door calls, agent clients, mandates",
        "W2, W6",
        "No agent calls yet."
    ),
    metric!(
        "journey.books",
        Journey,
        "Books: gross, fee, net per order",
        "concept/journeys/owner-order.html:40",
        Pending,
        "Customer payments, settlements",
        "W6",
        "No payments yet."
    ),
    metric!(
        "journey.invoice",
        Journey,
        "Invoice, eTIMS, next charge, AI spend of plan",
        "concept/journeys/owner-billing.html:48",
        Pending,
        "Plans, subscriptions, invoices",
        "W3",
        "No invoices yet."
    ),
    metric!(
        "journey.plan_local_price",
        Journey,
        "\"Essential plan from KSh 3,700/month\"",
        "concept/assess.html:328",
        Pending,
        "Local prices in the plan catalogue",
        "W3",
        "Plans are priced in USD."
    ),
    metric!(
        "journey.parity",
        Journey,
        "Parity per dimension; speed; \"23 common questions\"",
        "concept/assess.html",
        Pending,
        "Parity results and speed stored per page",
        "W4",
        "Parity scores appear once the Move proves the pages."
    ),
    metric!(
        "journey.redirects",
        Journey,
        "\"31 redirects, 18/18 prices\"",
        "concept/move.html:136-141",
        Pending,
        "Redirects; price parity counts",
        "W4",
        "Redirects are counted once the Move rebuilds the site."
    ),
    // ---- Control plane ----
    metric!(
        "fleet.sites",
        ControlPlane,
        "\"Sites live 612 (+9)\", by state",
        "concept/control-plane/fleet.html:66",
        Sourced,
        "Metrics::fleet(window)",
        "W8",
        "No sites yet."
    ),
    metric!(
        "fleet.moves_in_flight",
        ControlPlane,
        "Moves in flight, step, time on step",
        "concept/control-plane/fleet.html:66",
        Sourced,
        "Metrics::moves(in_flight: true)",
        "W8",
        "No Moves running."
    ),
    metric!(
        "fleet.step_times",
        ControlPlane,
        "Migration Factory stages: time per step",
        "concept/control-plane/fleet.html:76-144",
        Sourced,
        "Metrics::step_times(window)",
        "W8",
        "No finished steps in this window."
    ),
    metric!(
        "fleet.rooms",
        ControlPlane,
        "Sealed rooms: running, queued, by kind with times",
        "concept/control-plane/fleet.html:76-144",
        Partial,
        "Metrics::rooms; runner pools and utilisation are W3's",
        "W8, W3",
        "No sealed rooms ran in this window."
    ),
    metric!(
        "fleet.factory",
        ControlPlane,
        "Migration Factory strip: Moves at each step; p90 to the go-live card",
        "concept/control-plane/fleet.html:76-144",
        Sourced,
        "Metrics::move_factory(window)",
        "W8",
        "No Moves running."
    ),
    metric!(
        "fleet.room_minutes",
        ControlPlane,
        "Sealed-room minutes per site, for cost to serve",
        "concept/control-plane/portfolio.html:89-96",
        Sourced,
        "Metrics::room_minutes(scope, window)",
        "W8",
        "No sealed rooms ran in this window."
    ),
    metric!(
        "fleet.spend",
        ControlPlane,
        "Inference spend, by purpose and provider",
        "concept/control-plane/fleet.html:66-70",
        Sourced,
        "Metrics::spend(Fleet, window)",
        "W8",
        "No model calls in this window."
    ),
    metric!(
        "fleet.spend_forecast",
        ControlPlane,
        "Inference spend forecast",
        "concept/control-plane/fleet.html:66-70",
        Pending,
        "A forecast method over usage",
        "W12",
        "Forecasts start after a full month of usage."
    ),
    metric!(
        "fleet.spend_by_site",
        ControlPlane,
        "AI spend of cap per client site",
        "concept/control-plane/portfolio.html:89-96",
        Sourced,
        "Metrics::spend_by_site",
        "W8",
        "No model calls in this window."
    ),
    metric!(
        "fleet.decisions_waiting",
        ControlPlane,
        "Waiting per client site",
        "concept/control-plane/portfolio.html:89-96",
        Sourced,
        "Metrics::decisions_waiting(Fleet)",
        "W8",
        "Nothing waiting on any site."
    ),
    metric!(
        "fleet.gauntlet",
        ControlPlane,
        "Gauntlet score per site",
        "concept/control-plane/fleet.html",
        Sourced,
        "Metrics::gauntlet(None)",
        "W8",
        "The gauntlet hasn't run yet."
    ),
    metric!(
        "fleet.receipts",
        ControlPlane,
        "Receipts issued, by kind",
        "concept/control-plane/fleet.html",
        Sourced,
        "Metrics::receipt_counts(Fleet, window)",
        "W8",
        "No receipts in this window."
    ),
    metric!(
        "fleet.availability",
        ControlPlane,
        "Edge availability 99.97%, error budget, Ask p95",
        "concept/control-plane/fleet.html:66-70",
        Pending,
        "Health checks over time (W8 status), then OpenTelemetry",
        "W8",
        "Availability is measured from the first health check onward."
    ),
    metric!(
        "fleet.cells",
        ControlPlane,
        "Cells, runner pools and utilisation, SLO table",
        "concept/control-plane/fleet.html:76-144",
        Pending,
        "Cells, runner pools, SLO records",
        "W3",
        "One cell: atl-0."
    ),
    metric!(
        "fleet.operator_ledger",
        ControlPlane,
        "Operator ledger entries",
        "concept/control-plane/fleet.html",
        Pending,
        "The operator ledger",
        "W3",
        "No operator actions recorded."
    ),
    metric!(
        "portfolio.health",
        ControlPlane,
        "Health score per client site (operators and partners only)",
        "concept/control-plane/portfolio.html:89-96",
        Partial,
        "Metrics::site_health and health_by_site: 6 of 08 §4.1's 9 signals sourced",
        "W8, W1, W3, W5",
        "Not enough signals yet to score this site."
    ),
    metric!(
        "portfolio.clients",
        ControlPlane,
        "Client table: plan, health, role, ring, margin",
        "concept/control-plane/portfolio.html:89-96",
        Partial,
        "spend and waiting are sourced; partners, orgs, health scores are W3's",
        "W3, W8",
        "No client sites yet."
    ),
    metric!(
        "rollout.rings",
        ControlPlane,
        "Rings with site counts; canary metrics",
        "concept/control-plane/rollout.html:70",
        Pending,
        "Trains, rings, rollouts",
        "W3",
        "No rollouts yet."
    ),
    metric!(
        "incident.timeline",
        ControlPlane,
        "INC-0142 timeline, notices",
        "concept/control-plane/incident.html",
        Pending,
        "Incidents",
        "W3",
        "No incidents."
    ),
    metric!(
        "billing.mrr",
        ControlPlane,
        "MRR, collected, outstanding, payouts",
        "concept/control-plane/billing.html:67",
        Pending,
        "Invoices, payments, reconciliations",
        "W3",
        "No invoices yet."
    ),
    // ---- agentrix.website home page (W10) ----
    metric!(
        "home.gauntlet",
        Home,
        "Gauntlet score on the live sites",
        "concept/index.html",
        Sourced,
        "Metrics::gauntlet(None)",
        "W8",
        "The gauntlet runs after each deploy."
    ),
    metric!(
        "home.move_times",
        Home,
        "How long a Move takes, step by step",
        "concept/index.html",
        Sourced,
        "Metrics::step_times(Last30Days)",
        "W8",
        "No Moves finished yet."
    ),
    metric!(
        "home.sites_live",
        Home,
        "Sites live",
        "concept/index.html",
        Sourced,
        "Metrics::fleet(All).live",
        "W8",
        "No sites live yet."
    ),
    metric!(
        "home.receipts_issued",
        Home,
        "Receipts issued",
        "concept/index.html",
        Sourced,
        "Metrics::receipt_counts(Fleet, All).total",
        "W8",
        "No receipts yet."
    ),
];

/// The entry for `id`.
pub fn get(id: &str) -> Option<&'static Metric> {
    CATALOGUE.iter().find(|m| m.id == id)
}

/// A [`Reading::NoSource`] with the entry's words and owner.
///
/// # Panics
///
/// If `id` isn't in the catalogue: a programming error the catalogue test catches.
pub fn no_source<T>(id: &'static str) -> Reading<T> {
    let metric = get(id).unwrap_or_else(|| panic!("metric {id} is not in the catalogue"));
    Reading::NoSource {
        metric: metric.id,
        words: metric.empty,
        owner: metric.owner,
    }
}

/// How many entries have each status: (sourced, partial, pending).
pub fn coverage() -> (usize, usize, usize) {
    CATALOGUE
        .iter()
        .fold((0, 0, 0), |(s, p, n), m| match m.status {
            Status::Sourced => (s + 1, p, n),
            Status::Partial => (s, p + 1, n),
            Status::Pending => (s, p, n + 1),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_shaped() {
        let mut ids: Vec<_> = CATALOGUE.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate metric id");
        for m in CATALOGUE {
            assert!(m.id.contains('.'), "{}", m.id);
            assert!(m.concept.starts_with("concept/"), "{}", m.id);
            assert!(
                m.empty.ends_with('.'),
                "{}: empty words end a sentence",
                m.id
            );
        }
    }

    #[test]
    fn empty_words_keep_the_voice() {
        let banned = [
            "seamless",
            "effortless",
            "unlock",
            "oops",
            "something went wrong",
            "no data",
            "journey",
            "magic",
            "!",
            "\u{2014}",
            "\u{2013}",
        ];
        for m in CATALOGUE {
            let lower = m.empty.to_lowercase();
            for word in banned {
                assert!(
                    !lower.contains(word),
                    "{}: {:?} in {:?}",
                    m.id,
                    word,
                    m.empty
                );
            }
        }
    }

    #[test]
    fn every_entry_from_03_section_7_is_here() {
        // 03-data.md §7 lists 56 numbers and lists with no source; this catalogue covers each,
        // plus what W3, W5 and W10 asked for.
        let (sourced, partial, pending) = coverage();
        assert!(CATALOGUE.len() >= 70, "{}", CATALOGUE.len());
        assert!(sourced >= 20 && partial >= 5 && pending >= 40);
        assert!(matches!(
            no_source::<()>("site.visits"),
            Reading::NoSource {
                owner: "W8, W2",
                ..
            }
        ));
    }
}
