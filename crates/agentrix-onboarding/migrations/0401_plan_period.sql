-- W4 onboarding: the plan is paid monthly or 12 months up front (at W3's catalogue discount).
-- Additive: an existing choice, and one an older binary writes, is monthly. On a live plan the
-- owner's choice can differ from the subscription's period: that's a switch waiting for the
-- next renewal, shown on Billing.
ALTER TABLE onboarding_plan_choices ADD COLUMN period TEXT NOT NULL DEFAULT 'monthly'
    CHECK (period IN ('monthly', 'annual'));
