//! The catalogue: every message the platform sends, who gets it, on which channels, and why.
//!
//! Each entry points at one template file under `templates/` (the words) and one example
//! context (`*.json` beside it) that the tests render and the preview gallery shows. Adding a
//! message is one line here plus those two files; a missing file fails the build.

use serde::Serialize;

/// Who a message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// A customer of a site, in the site's name.
    Customer,
    /// Someone who subscribed to a site's digest.
    Subscriber,
    /// The owner of a site, from Agentrix.
    Owner,
    /// Someone invited to help run a site (manager, viewer, agency, billing).
    Teammate,
    /// Someone who asked Agentrix itself for something: a website check, the weekly fix.
    Prospect,
}

impl Audience {
    /// The word used in the console.
    pub fn label(self) -> &'static str {
        match self {
            Self::Customer => "Customer",
            Self::Subscriber => "Subscriber",
            Self::Owner => "Owner",
            Self::Teammate => "Teammate",
            Self::Prospect => "Asked Agentrix",
        }
    }
}

/// What kind of message it is, which decides the rules it follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Codes and account alerts: always sent, never held for quiet hours, no unsubscribe.
    Security,
    /// What someone asked for or bought: always sent, no unsubscribe.
    Transactional,
    /// Notices that run the business: follow the person's channel choices and quiet hours.
    Operational,
    /// Something people signed up to (the digest, offers): one-tap unsubscribe on every copy.
    Digest,
}

impl Kind {
    /// The word used in the console.
    pub fn label(self) -> &'static str {
        match self {
            Self::Security => "Security, always sent",
            Self::Transactional => "Always sent",
            Self::Operational => "You can turn this off",
            Self::Digest => "The reader asked for it",
        }
    }
}

/// How urgent a message is, as the attention manager routes it (07 §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Site down, payment anomaly, incident: interrupts quiet hours until acknowledged.
    P1,
    /// Within the hour, outside quiet hours.
    P2,
    /// Whenever is convenient: the brief or the weekly review.
    P3,
}

/// Where a message can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Email: plain text and HTML, approves nothing (ADR-014).
    Email,
    /// Web push to the console app: says only that something is waiting.
    Push,
    /// WhatsApp or Telegram: may decline, ask, pause and stop; approving always opens the console.
    Chat,
}

impl Channel {
    /// Every channel, in the order the console lists them.
    pub const ALL: [Channel; 3] = [Channel::Email, Channel::Push, Channel::Chat];

    /// The stored and URL form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Push => "push",
            Self::Chat => "chat",
        }
    }

    /// The word used in the console.
    pub fn label(self) -> &'static str {
        match self {
            Self::Email => "Email",
            Self::Push => "Phone notifications",
            Self::Chat => "Chat",
        }
    }

    /// Parses the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "push" => Some(Self::Push),
            "chat" => Some(Self::Chat),
            _ => None,
        }
    }
}

/// The family a message belongs to, for the console's grouping and the board's tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Customer sign-in, codes and account alerts.
    CustomerAccount,
    /// Customer orders, requests, payments and refunds.
    CustomerOrders,
    /// Customer data rights.
    CustomerData,
    /// The news digest.
    Digest,
    /// Owner sign-in, recovery and security alerts.
    OwnerAccount,
    /// Owner onboarding: preview, claim, the Move, go-live.
    OwnerOnboarding,
    /// Owner daily running: brief, decisions, alarms, freeze, budget, orders.
    OwnerOperations,
    /// Plans, invoices and payment problems.
    OwnerBilling,
    /// Platform notices, support access, agencies and teammates.
    OwnerPlatform,
    /// Owner data rights and incidents.
    OwnerData,
    /// What people ask Agentrix for on agentrix.website: a website check, the weekly fix.
    Prospect,
}

impl Family {
    /// The heading used in the console and the gallery.
    pub fn label(self) -> &'static str {
        match self {
            Self::CustomerAccount => "Customer sign-in and security",
            Self::CustomerOrders => "Customer orders",
            Self::CustomerData => "Customer data",
            Self::Digest => "Digest",
            Self::OwnerAccount => "Owner sign-in and security",
            Self::OwnerOnboarding => "Moving in",
            Self::OwnerOperations => "Running the site",
            Self::OwnerBilling => "Plan and billing",
            Self::OwnerPlatform => "Platform, support and team",
            Self::OwnerData => "Owner data",
            Self::Prospect => "Website checks and the weekly fix",
        }
    }
}

/// One message in the catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDef {
    /// Stable id, `audience.name`, used by callers and stored in the outbox.
    pub id: &'static str,
    /// Who gets it.
    pub audience: Audience,
    /// Its family.
    pub family: Family,
    /// Which rules it follows.
    pub kind: Kind,
    /// How urgent it is.
    pub priority: Priority,
    /// The channels it has versions for.
    pub channels: &'static [Channel],
    /// What makes the platform send it, in words.
    pub trigger: &'static str,
    /// The workstream whose code sends it.
    pub raised_by: &'static str,
    /// Context paths that are secret (one-time codes): never stored, never shown.
    pub secret: &'static [&'static str],
    /// The template source.
    #[serde(skip)]
    pub source: &'static str,
    /// An example context, as JSON.
    #[serde(skip)]
    pub example: &'static str,
}

use Audience::{Customer, Owner, Prospect, Subscriber, Teammate};
use Channel::{Chat, Email, Push};
use Kind::{Digest as Signed, Operational, Security, Transactional};
use Priority::{P1, P2, P3};

macro_rules! msg {
    ($id:literal, $file:literal, $aud:expr, $fam:ident, $kind:expr, $prio:expr, [$($ch:expr),+], $by:literal, $trigger:literal $(, secret = [$($s:literal),*])?) => {
        MessageDef {
            id: $id,
            audience: $aud,
            family: Family::$fam,
            kind: $kind,
            priority: $prio,
            channels: &[$($ch),+],
            trigger: $trigger,
            raised_by: $by,
            secret: &[$($($s),*)?],
            source: include_str!(concat!("../templates/", $file, ".txt")),
            example: include_str!(concat!("../templates/", $file, ".json")),
        }
    };
}

static CATALOGUE: &[MessageDef] = &[
    // Customer sign-in and security (W6 sends these).
    msg!(
        "customer.signin_code",
        "customer/signin_code",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A customer signs in with their email and asks for a code",
        secret = ["code"]
    ),
    msg!(
        "customer.create_code",
        "customer/create_code",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "Someone starts making an account on a site and proves their email",
        secret = ["code"]
    ),
    msg!(
        "customer.verify_email",
        "customer/verify_email",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A new account confirms its email before the first order",
        secret = ["code"]
    ),
    msg!(
        "customer.recovery_code",
        "customer/recovery_code",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A customer asks for a code to get back into their account",
        secret = ["code"]
    ),
    msg!(
        "customer.recovery_code_used",
        "customer/recovery_code_used",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "Someone signs in to a customer's account with one of their recovery codes"
    ),
    msg!(
        "customer.order_lookup_code",
        "customer/order_lookup_code",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email, Chat],
        "W6",
        "A guest asks to see the details of an order and proves it's theirs",
        secret = ["code"]
    ),
    msg!(
        "customer.account_created",
        "customer/account_created",
        Customer,
        CustomerAccount,
        Transactional,
        P3,
        [Email],
        "W6",
        "A customer creates an account on a site"
    ),
    msg!(
        "customer.new_signin",
        "customer/new_signin",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A device the customer hasn't used before signs in to their account"
    ),
    msg!(
        "customer.details_changed",
        "customer/details_changed",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A customer's email, phone or delivery address changes; sent to the old address"
    ),
    msg!(
        "customer.signin_method_changed",
        "customer/signin_method_changed",
        Customer,
        CustomerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "A passkey is added or removed, or new recovery codes are made"
    ),
    // Customer orders (W2 and W6 send these).
    msg!(
        "customer.order_received",
        "customer/order_received",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "A customer pays for an order"
    ),
    msg!(
        "customer.request_held",
        "customer/request_held",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "A custom order or request waits for the owner's OK"
    ),
    msg!(
        "customer.request_approved",
        "customer/request_approved",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "The owner approves a request and a payment request is ready"
    ),
    msg!(
        "customer.request_declined",
        "customer/request_declined",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "The owner declines a request"
    ),
    msg!(
        "customer.request_change_suggested",
        "customer/request_change_suggested",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "The owner suggests a change to a request instead of approving it"
    ),
    msg!(
        "customer.request_lapsed",
        "customer/request_lapsed",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "A request's time runs out before the owner answers"
    ),
    msg!(
        "customer.payment_receipt",
        "customer/payment_receipt",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email],
        "W6",
        "PayPal confirms a payment and its signed receipt is written"
    ),
    msg!(
        "customer.order_status",
        "customer/order_status",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "The owner or a WebAgent moves an order to its next step"
    ),
    msg!(
        "customer.change_answered",
        "customer/change_answered",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "A change the customer asked for is approved or declined"
    ),
    msg!(
        "customer.refund_issued",
        "customer/refund_issued",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W6",
        "The Cashier refunds money within the owner's rule, or the owner approves a refund"
    ),
    msg!(
        "customer.refund_held",
        "customer/refund_held",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W6",
        "A refund is above what the Cashier may give without the owner"
    ),
    msg!(
        "customer.reply_from_owner",
        "customer/reply_from_owner",
        Customer,
        CustomerOrders,
        Transactional,
        P2,
        [Email, Chat],
        "W2",
        "The owner answers a question a desk passed on"
    ),
    // Customer data rights (W6).
    msg!(
        "customer.export_ready",
        "customer/export_ready",
        Customer,
        CustomerData,
        Security,
        P2,
        [Email],
        "W6",
        "A customer's data export is ready to download"
    ),
    msg!(
        "customer.erasure_done",
        "customer/erasure_done",
        Customer,
        CustomerData,
        Security,
        P2,
        [Email],
        "W6",
        "A customer's account and data are erased; sent before the address itself goes"
    ),
    // Digest (W2 sends these for Tana Valley Times).
    msg!(
        "digest.confirm",
        "digest/confirm",
        Subscriber,
        Digest,
        Transactional,
        P2,
        [Email],
        "W2",
        "Someone asks for the digest; nothing is sent until they confirm (double opt-in)"
    ),
    msg!(
        "digest.weekly",
        "digest/weekly",
        Subscriber,
        Digest,
        Signed,
        P3,
        [Email],
        "W2",
        "Friday at 07:00, for each confirmed subscriber"
    ),
    msg!(
        "digest.stopped",
        "digest/stopped",
        Subscriber,
        Digest,
        Transactional,
        P3,
        [Email],
        "W2",
        "A subscriber stops the digest; their topics are deleted"
    ),
    // Owner sign-in and security (W6).
    msg!(
        "owner.verify_email",
        "owner/verify_email",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "An owner creates an account and confirms their email",
        secret = ["code"]
    ),
    msg!(
        "owner.signin_code",
        "owner/signin_code",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "An owner signs in with an email code instead of a passkey",
        secret = ["code"]
    ),
    msg!(
        "owner.code_signin",
        "owner/code_signin",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email, Push, Chat],
        "W6",
        "An owner signed in with an email code or a recovery code; every other channel hears about it"
    ),
    msg!(
        "owner.recovery_code",
        "owner/recovery_code",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email],
        "W6",
        "An owner who lost their passkey or authenticator starts recovery by email",
        secret = ["code"]
    ),
    msg!(
        "owner.recovery_started",
        "owner/recovery_started",
        Owner,
        OwnerAccount,
        Security,
        P1,
        [Email, Push, Chat],
        "W6",
        "Account recovery starts; the 24-hour hold begins and every other channel hears about it"
    ),
    msg!(
        "owner.new_device",
        "owner/new_device",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email, Push],
        "W6",
        "A device the owner hasn't used before signs in"
    ),
    msg!(
        "owner.signin_method_changed",
        "owner/signin_method_changed",
        Owner,
        OwnerAccount,
        Security,
        P2,
        [Email, Push],
        "W6",
        "A passkey is added or removed, the authenticator is reset, or new recovery codes are made"
    ),
    // Owner onboarding (W4).
    msg!(
        "owner.claim_reminder",
        "owner/claim_reminder",
        Owner,
        OwnerOnboarding,
        Operational,
        P3,
        [Email, Chat],
        "W4",
        "A domain claim has waited 24 hours for its DNS record"
    ),
    msg!(
        "owner.move_update",
        "owner/move_update",
        Owner,
        OwnerOnboarding,
        Operational,
        P2,
        [Email, Push, Chat],
        "W4",
        "The Move starts, needs the owner, or stops"
    ),
    msg!(
        "owner.live",
        "owner/live",
        Owner,
        OwnerOnboarding,
        Operational,
        P2,
        [Email, Chat],
        "W4",
        "The site goes live on the platform; the undo window starts"
    ),
    msg!(
        "owner.fallback_retiring",
        "owner/fallback_retiring",
        Owner,
        OwnerOnboarding,
        Operational,
        P3,
        [Email, Chat],
        "W4",
        "Day 23 after go-live: the old site on standby retires in 7 days"
    ),
    // Owner daily running (W1, W5, W2, W8).
    msg!(
        "owner.morning_brief",
        "owner/morning_brief",
        Owner,
        OwnerOperations,
        Operational,
        P3,
        [Email, Push, Chat],
        "W1",
        "Quiet hours end (07:00 by default) and the Chief WebAgent condenses the night"
    ),
    msg!(
        "owner.weekly_review",
        "owner/weekly_review",
        Owner,
        OwnerOperations,
        Operational,
        P3,
        [Email],
        "W1",
        "Monday morning: the week's work, spend, earned autonomy and mission reports"
    ),
    msg!(
        "owner.decision_waiting",
        "owner/decision_waiting",
        Owner,
        OwnerOperations,
        Operational,
        P2,
        [Push, Chat],
        "W5",
        "A decision is waiting for you (never during quiet hours)"
    ),
    msg!(
        "owner.decision_expiring",
        "owner/decision_expiring",
        Owner,
        OwnerOperations,
        Operational,
        P2,
        [Push, Chat],
        "W5",
        "A waiting decision has less than an hour before it expires"
    ),
    msg!(
        "owner.alarm",
        "owner/alarm",
        Owner,
        OwnerOperations,
        Operational,
        P1,
        [Email, Push, Chat],
        "W8",
        "Site down, a payment anomaly or an incident: rings through quiet hours until acknowledged"
    ),
    msg!(
        "owner.frozen",
        "owner/frozen",
        Owner,
        OwnerOperations,
        Security,
        P1,
        [Email, Push, Chat],
        "W6",
        "Someone freezes every WebAgent on the site, or unfreezes them"
    ),
    msg!(
        "owner.budget_reached",
        "owner/budget_reached",
        Owner,
        OwnerOperations,
        Operational,
        P2,
        [Email, Push, Chat],
        "W8",
        "The site's monthly AI budget runs out and WebAgents rest"
    ),
    msg!(
        "owner.new_order",
        "owner/new_order",
        Owner,
        OwnerOperations,
        Operational,
        P2,
        [Email, Push, Chat],
        "W2",
        "A customer pays for an order"
    ),
    msg!(
        "owner.question_passed",
        "owner/question_passed",
        Owner,
        OwnerOperations,
        Operational,
        P2,
        [Email, Push, Chat],
        "W2",
        "A desk can't answer from the site and passes a visitor's question to the owner"
    ),
    msg!(
        "owner.mission_report",
        "owner/mission_report",
        Owner,
        OwnerOperations,
        Operational,
        P3,
        [Email],
        "W1",
        "A mission ends, or its weekly report is due"
    ),
    // Owner billing (W6).
    msg!(
        "owner.plan_started",
        "owner/plan_started",
        Owner,
        OwnerBilling,
        Transactional,
        P3,
        [Email],
        "W6",
        "The plan's first charge, on go-live day"
    ),
    msg!(
        "owner.invoice",
        "owner/invoice",
        Owner,
        OwnerBilling,
        Transactional,
        P3,
        [Email],
        "W6",
        "An invoice is issued, or a payment for one is received"
    ),
    msg!(
        "owner.payment_failed",
        "owner/payment_failed",
        Owner,
        OwnerBilling,
        Transactional,
        P2,
        [Email, Chat],
        "W6",
        "A plan payment fails: day 0, 3 and 7; read-only at day 14; closure notice at day 30"
    ),
    msg!(
        "owner.plan_changed",
        "owner/plan_changed",
        Owner,
        OwnerBilling,
        Transactional,
        P3,
        [Email],
        "W6",
        "The plan changes, is cancelled, or the site's closing date is set"
    ),
    // Owner platform, support and team (W3, W6).
    msg!(
        "owner.platform_notice",
        "owner/platform_notice",
        Owner,
        OwnerPlatform,
        Operational,
        P2,
        [Email],
        "W3",
        "Zavora posts a notice about the platform, or places or lifts a hold on the site"
    ),
    msg!(
        "owner.support_access",
        "owner/support_access",
        Owner,
        OwnerPlatform,
        Security,
        P2,
        [Email, Push],
        "W3",
        "Support asks to see the site, or a granted access starts or ends"
    ),
    msg!(
        "teammate.invite",
        "teammate/invite",
        Teammate,
        OwnerPlatform,
        Transactional,
        P2,
        [Email],
        "W6",
        "An owner or agency invites someone to help run a site"
    ),
    msg!(
        "owner.role_changed",
        "owner/role_changed",
        Owner,
        OwnerPlatform,
        Security,
        P2,
        [Email],
        "W6",
        "A teammate or agency is given a role, or has one removed"
    ),
    // Owner data (W8, W6).
    msg!(
        "owner.export_ready",
        "owner/export_ready",
        Owner,
        OwnerData,
        Security,
        P2,
        [Email],
        "W8",
        "The site's full export is ready (the link lasts 7 days)"
    ),
    msg!(
        "owner.erasure_done",
        "owner/erasure_done",
        Owner,
        OwnerData,
        Operational,
        P3,
        [Email],
        "W8",
        "A customer's erasure finishes across every store"
    ),
    msg!(
        "owner.data_incident",
        "owner/data_incident",
        Owner,
        OwnerData,
        Security,
        P1,
        [Email],
        "W6",
        "A data incident that affects the site, within 72 hours of finding it"
    ),
    // People who asked Agentrix itself (W10's agentrix.website).
    msg!(
        "lead.report",
        "lead/report",
        Prospect,
        Prospect,
        Transactional,
        P3,
        [Email, Chat],
        "W10",
        "Someone asks for their website check report on agentrix.website"
    ),
    msg!(
        "newsletter.confirm",
        "newsletter/confirm",
        Prospect,
        Prospect,
        Transactional,
        P3,
        [Email, Chat],
        "W10",
        "Someone asks for the weekly fix; nothing is sent until they confirm"
    ),
    msg!(
        "newsletter.issue",
        "newsletter/issue",
        Prospect,
        Prospect,
        Signed,
        P3,
        [Email, Chat],
        "W10",
        "Every week, to each reader who confirmed"
    ),
];

/// Every message, in catalogue order.
pub fn all() -> &'static [MessageDef] {
    CATALOGUE
}

/// The message with this id.
pub fn get(id: &str) -> Option<&'static MessageDef> {
    CATALOGUE.iter().find(|d| d.id == id)
}

impl MessageDef {
    /// The example context, parsed.
    pub fn example_context(&self) -> serde_json::Value {
        serde_json::from_str(self.example)
            .unwrap_or_else(|e| panic!("example for {} isn't JSON: {e}", self.id))
    }
}
