-- Platform billing (W3, agentrix-billing): plans paid monthly or 12 months up front.
-- Additive: an existing row, and a row an older binary writes, is monthly. price_cents stays
-- what one charge is: the monthly price, or the 12-month total less the catalogue's discount.
ALTER TABLE subscriptions ADD COLUMN period TEXT NOT NULL DEFAULT 'monthly'
    CHECK (period IN ('monthly', 'annual'));
