-- W4 onboarding (specs/platform/03-data.md catalogue: owner agentrix-onboarding).
-- The plan an owner chose during setup, before the PayPal subscription is approved at go-live.
-- One row per site; choosing again replaces it. Refers to sites and accounts by id only.
CREATE TABLE IF NOT EXISTS onboarding_plan_choices (
    site_id        TEXT PRIMARY KEY,
    account        TEXT NOT NULL,
    plan_id        TEXT NOT NULL,
    ai_budget_usd  INTEGER NOT NULL CHECK (ai_budget_usd >= 0),
    chosen_at      TEXT NOT NULL
);
