-- agentrix-records (W2, migration range 0200-0299). One database file, records.db.
-- Money is in minor units of the row's currency (KES: KSh 450 is 45000). Times are unix seconds.
-- Every table is keyed by `site`, the site's slug (`kilele-bakery-sample`).

CREATE TABLE rec_sites (
    site TEXT PRIMARY KEY,
    host TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    archetype TEXT NOT NULL,
    currency TEXT NOT NULL,
    utc_offset_min INTEGER NOT NULL,
    origin TEXT NOT NULL,
    facts TEXT NOT NULL,
    seed_version TEXT NOT NULL,
    anchor_at INTEGER NOT NULL,
    loaded_at INTEGER NOT NULL
);

CREATE TABLE rec_products (
    site TEXT NOT NULL,
    sku TEXT NOT NULL,
    slug TEXT NOT NULL,
    section TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    family TEXT,
    size TEXT,
    price_minor INTEGER NOT NULL,
    attrs TEXT NOT NULL,
    status TEXT NOT NULL,
    sort INTEGER NOT NULL,
    PRIMARY KEY (site, sku)
);
CREATE UNIQUE INDEX rec_products_slug ON rec_products (site, slug);

-- What is made or stocked for a day. Bakeries plan per local day; shops use day '' (always).
CREATE TABLE rec_stock (
    site TEXT NOT NULL,
    sku TEXT NOT NULL,
    day TEXT NOT NULL,
    planned INTEGER NOT NULL,
    next_batch_at INTEGER,
    back_at INTEGER,
    as_of INTEGER NOT NULL,
    source TEXT NOT NULL,
    PRIMARY KEY (site, sku, day)
);

-- The daily bake plan the stock rows are made from each morning.
CREATE TABLE rec_bake_plan (
    site TEXT NOT NULL,
    sku TEXT NOT NULL,
    planned INTEGER NOT NULL,
    batches TEXT NOT NULL,
    PRIMARY KEY (site, sku)
);

CREATE TABLE rec_price_lists (
    site TEXT NOT NULL,
    version TEXT NOT NULL,
    currency TEXT NOT NULL,
    status TEXT NOT NULL,
    usd_rate_minor INTEGER NOT NULL,
    valid_from INTEGER NOT NULL,
    valid_until INTEGER NOT NULL,
    source TEXT NOT NULL,
    PRIMARY KEY (site, version)
);

CREATE TABLE rec_price_rules (
    site TEXT NOT NULL,
    version TEXT NOT NULL,
    code TEXT NOT NULL,
    grp TEXT NOT NULL,
    label TEXT NOT NULL,
    amount_minor INTEGER NOT NULL,
    meta TEXT NOT NULL,
    sort INTEGER NOT NULL,
    PRIMARY KEY (site, version, code)
);

CREATE TABLE rec_zones (
    site TEXT NOT NULL,
    zone TEXT NOT NULL,
    label TEXT NOT NULL,
    fee_minor INTEGER NOT NULL,
    areas TEXT NOT NULL,
    note TEXT NOT NULL,
    PRIMARY KEY (site, zone)
);

CREATE TABLE rec_slots (
    site TEXT NOT NULL,
    slot_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    starts_at INTEGER NOT NULL,
    ends_at INTEGER NOT NULL,
    capacity INTEGER NOT NULL,
    PRIMARY KEY (site, slot_id)
);
CREATE INDEX rec_slots_time ON rec_slots (site, starts_at);

CREATE TABLE rec_customers (
    site TEXT NOT NULL,
    customer_id TEXT NOT NULL,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    phone TEXT,
    kind TEXT NOT NULL,
    place TEXT,
    sample INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, customer_id)
);
CREATE UNIQUE INDEX rec_customers_email ON rec_customers (site, email);

CREATE TABLE rec_carts (
    site TEXT NOT NULL,
    cart_id TEXT NOT NULL,
    customer_id TEXT,
    lines TEXT NOT NULL,
    slot_id TEXT,
    zone TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (site, cart_id)
);

CREATE TABLE rec_orders (
    site TEXT NOT NULL,
    order_id TEXT NOT NULL,
    customer_id TEXT,
    contact_name TEXT NOT NULL,
    contact_email TEXT,
    contact_phone TEXT,
    channel TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    subtotal_minor INTEGER NOT NULL,
    delivery_minor INTEGER NOT NULL,
    total_minor INTEGER NOT NULL,
    currency TEXT NOT NULL,
    usd_rate_minor INTEGER NOT NULL,
    slot_id TEXT,
    zone TEXT,
    fulfilment TEXT NOT NULL,
    recipient TEXT,
    note TEXT,
    spec TEXT,
    price_list TEXT,
    quote_valid_until INTEGER,
    agent TEXT,
    idempotency_key TEXT,
    sample INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (site, order_id)
);
CREATE INDEX rec_orders_customer ON rec_orders (site, customer_id);
CREATE UNIQUE INDEX rec_orders_idem ON rec_orders (site, idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE TABLE rec_order_lines (
    site TEXT NOT NULL,
    order_id TEXT NOT NULL,
    line_no INTEGER NOT NULL,
    sku TEXT,
    name TEXT NOT NULL,
    qty INTEGER NOT NULL,
    unit_minor INTEGER NOT NULL,
    rule_code TEXT,
    amount_minor INTEGER NOT NULL,
    PRIMARY KEY (site, order_id, line_no)
);

CREATE TABLE rec_order_events (
    site TEXT NOT NULL,
    order_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    step TEXT NOT NULL,
    at INTEGER NOT NULL,
    actor TEXT NOT NULL,
    note TEXT,
    PRIMARY KEY (site, order_id, seq)
);

CREATE TABLE rec_payments (
    site TEXT NOT NULL,
    payment_id TEXT NOT NULL,
    order_id TEXT NOT NULL,
    rail TEXT NOT NULL,
    provider_ref TEXT,
    amount_minor INTEGER NOT NULL,
    currency TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    PRIMARY KEY (site, payment_id)
);

CREATE TABLE rec_refunds (
    site TEXT NOT NULL,
    refund_id TEXT NOT NULL,
    order_id TEXT NOT NULL,
    amount_minor INTEGER NOT NULL,
    reason TEXT NOT NULL,
    rule TEXT,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, refund_id)
);

CREATE TABLE rec_holds (
    site TEXT NOT NULL,
    hold_id TEXT NOT NULL,
    sku TEXT NOT NULL,
    qty INTEGER NOT NULL,
    day TEXT NOT NULL,
    until_at INTEGER NOT NULL,
    name TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, hold_id)
);

CREATE TABLE rec_bookings (
    site TEXT NOT NULL,
    booking_id TEXT NOT NULL,
    service TEXT NOT NULL,
    starts_at INTEGER NOT NULL,
    customer_id TEXT,
    name TEXT NOT NULL,
    seats INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, booking_id)
);

CREATE TABLE rec_questions (
    site TEXT NOT NULL,
    question_id TEXT NOT NULL,
    desk TEXT NOT NULL,
    question TEXT NOT NULL,
    contact TEXT,
    page TEXT,
    state TEXT NOT NULL,
    answer TEXT,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, question_id)
);

CREATE TABLE rec_counters (
    site TEXT NOT NULL,
    name TEXT NOT NULL,
    next INTEGER NOT NULL,
    PRIMARY KEY (site, name)
);

CREATE TABLE rec_authors (
    site TEXT NOT NULL,
    author_id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    bio TEXT NOT NULL,
    PRIMARY KEY (site, author_id)
);

CREATE TABLE rec_stories (
    site TEXT NOT NULL,
    slug TEXT NOT NULL,
    section TEXT NOT NULL,
    kicker TEXT,
    title TEXT NOT NULL,
    standfirst TEXT NOT NULL,
    body TEXT NOT NULL,
    author_id TEXT NOT NULL,
    topic TEXT,
    entitlement TEXT NOT NULL,
    status TEXT NOT NULL,
    published_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (site, slug)
);
CREATE INDEX rec_stories_time ON rec_stories (site, published_at);

CREATE TABLE rec_corrections (
    site TEXT NOT NULL,
    story_slug TEXT NOT NULL,
    at INTEGER NOT NULL,
    what TEXT NOT NULL,
    note TEXT NOT NULL,
    PRIMARY KEY (site, story_slug, at)
);

CREATE TABLE rec_timeline (
    site TEXT NOT NULL,
    topic TEXT NOT NULL,
    at INTEGER NOT NULL,
    line TEXT NOT NULL,
    story_slug TEXT,
    kind TEXT NOT NULL,
    PRIMARY KEY (site, topic, at)
);

CREATE TABLE rec_subscribers (
    site TEXT NOT NULL,
    subscriber_id TEXT NOT NULL,
    customer_id TEXT NOT NULL,
    plan TEXT NOT NULL,
    state TEXT NOT NULL,
    source TEXT NOT NULL,
    since INTEGER NOT NULL,
    until_at INTEGER,
    PRIMARY KEY (site, subscriber_id)
);
CREATE UNIQUE INDEX rec_subscribers_customer ON rec_subscribers (site, customer_id);

CREATE TABLE rec_digest_subs (
    site TEXT NOT NULL,
    sub_id TEXT NOT NULL,
    email TEXT NOT NULL,
    topics TEXT NOT NULL,
    state TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    confirmed_at INTEGER,
    stopped_at INTEGER,
    PRIMARY KEY (site, sub_id)
);

-- Customer sign-in on the site's own host, for samples until W6's identity crate takes over.
CREATE TABLE rec_sessions (
    site TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    customer_id TEXT NOT NULL,
    device TEXT,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    ended_at INTEGER,
    PRIMARY KEY (site, token_hash)
);

CREATE TABLE rec_codes (
    site TEXT NOT NULL,
    email TEXT NOT NULL,
    purpose TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (site, email, purpose)
);
