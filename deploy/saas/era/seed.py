#!/usr/bin/env python3
"""Agentrix SaaS: seed both companies' books in the local Zavora ERA (hosted). Idempotent.

    deploy/saas/era/era.sh seed        (or: python3 -I seed.py --url … --state … --env …)

For "Agentrix Ltd" and "Kilele Bakery" it makes, once each:
  - the tenant and its owner login (logins are generated and kept in <state>/tenants.json, 0600;
    ERA's signup always makes a new tenant, so this logs in first and signs up only on 401);
  - USD as the base currency;
  - GL 1050 "PayPal Clearing - USD", the bank account "PayPal clearing" on it, and GL 8075
    "PayPal Fees";
  - the CRM switched on;
  - the price list (Agentrix: the WebAgent plans, monthly and 12-month, from agentrix-billing's
    plans.json prices), and out-of-scope expense items for vendor bills;
  - customers and vendors: each other, Vultr, Anthropic, a contractor and a referral partner,
    with credit limits;
  - a read-only login for judges, with ERA's own `Viewer` role (reads everything but payroll, HR,
    admin and the audit log; writes nothing): judge+agentrix@ and judge+kilele@agentrix.website.
    Its password is SAAS_ERA_DEMO_PASSWORD from --env when set, else generated once and kept in
    <state>/tenants.json under "judges". Never printed.

Matches by name everywhere (ERA has no unique names), so running it again changes nothing. Prints
ids, never secrets. Standard library only.
"""

import argparse
import json
import os
import secrets
import sys
import urllib.error
import urllib.request

COMPANIES = {
    "agentrix": {"name": "Agentrix Ltd", "email": "books+agentrix@agentrix.website", "merchant_env": "PAYPAL_MERCHANT_EMAIL",
                 "judge": "judge+agentrix@agentrix.website"},
    "kilele-bakery": {"name": "Kilele Bakery", "email": "books+kilele@agentrix.website", "merchant_env": "KILELE_PAYPAL_MERCHANT_EMAIL",
                      "judge": "judge+kilele@agentrix.website"},
}

JUDGE_ROLE = "Viewer"

CLEARING_CODE, FEES_CODE = "1050", "8075"

# The WebAgent plans (crates/agentrix-billing/data/plans.json): a 12-month plan is 12 months less 20%.
PLANS = [("Essential", 2900), ("Business", 7900), ("Commerce", 14900)]

COMMON_PRODUCTS = [
    # name, type, uom, price, sales account, purchase account
    ("General services", "Service", "Each", None, "5100", None),
    ("General expense", "Expense", "Each", None, None, "7900"),
    ("Cloud hosting", "Expense", "Month", None, None, "7350"),
    ("AI model usage", "Expense", "Each", None, None, "7350"),
    ("Contract services", "Expense", "Each", None, None, "7500"),
    ("Referral commission", "Expense", "Each", None, None, "7700"),
]

KILELE_PRODUCTS = [
    ("Bread and pastries", "Service", "Each", "45.00", "5000", None),
    ("Office catering", "Service", "Day", "120.00", "5100", None),
    ("WebAgent subscription", "Expense", "Month", None, None, "7350"),
]


def read_env(path):
    """KEY=value lines; a value starting with '#' is empty. Never printed."""
    out = {}
    if not path or not os.path.exists(path):
        return out
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.lstrip().startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            v = v.strip().strip("'\"")
            out[k.strip()] = "" if v.startswith("#") else v
    return out


class Era:
    def __init__(self, base):
        self.base = base.rstrip("/") + "/api/v1"
        self.token = None

    def call(self, method, path, body=None, ok404=False):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("Accept", "application/json")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if self.token:
            req.add_header("Authorization", "Bearer " + self.token)
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                raw = r.read().decode() or "null"
                return r.status, json.loads(raw)
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")
            try:
                payload = json.loads(text)
            except ValueError:
                payload = {"error": text}
            return e.code, payload

    def must(self, method, path, body=None):
        status, payload = self.call(method, path, body)
        if status >= 300:
            sys.exit(f"ERA {method} {path} answered {status}: {payload}")
        return payload


def ensure_tenant(era, key, spec, creds):
    era.token = None
    status, payload = era.call("POST", "/auth/login", {"email": creds["email"], "password": creds["password"]})
    if status == 401:
        payload = era.must("POST", "/auth/signup", {
            "organization_name": spec["name"],
            "organization_type": "limited_company",
            "email": creds["email"],
            "display_name": f"{spec['name']} books",
            "password": creds["password"],
            "with_sample_data": False,
            "plan": "business",
        })
        print(f"[{key}] signed up {spec['name']}")
    elif status >= 300:
        sys.exit(f"[{key}] login answered {status}: {payload}")
    era.token = payload["access_token"]
    creds["entity_id"] = payload["user"]["entity_id"]
    creds["user_id"] = payload["user"]["user_id"]


def ensure_usd(era, key):
    settings = era.must("GET", "/settings")
    if settings.get("base_currency") != "USD":
        era.must("PUT", "/settings", {"base_currency": "USD"})
        print(f"[{key}] base currency set to USD")


def ensure_accounts(era, key, merchant_email):
    accounts = {a["code"]: a for a in era.must("GET", "/accounts")}
    if CLEARING_CODE not in accounts:
        era.must("POST", "/accounts", {"code": CLEARING_CODE, "name": "PayPal Clearing - USD", "account_type": "Asset",
                                       "parent_code": "1000" if "1000" in accounts else None, "currency": "USD",
                                       "is_control": False, "tags": ["paypal"]})
        print(f"[{key}] GL {CLEARING_CODE} PayPal Clearing - USD")
    if FEES_CODE not in accounts:
        era.must("POST", "/accounts", {"code": FEES_CODE, "name": "PayPal Fees", "account_type": "Expense",
                                       "parent_code": "8050" if "8050" in accounts else None, "currency": "USD",
                                       "is_control": False, "tags": ["paypal"]})
        print(f"[{key}] GL {FEES_CODE} PayPal Fees")
    banks = era.must("GET", "/bank-accounts")
    bank = next((b for b in banks if b["name"] == "PayPal clearing"), None)
    if bank is None:
        bank = era.must("POST", "/bank-accounts", {"name": "PayPal clearing", "bank_name": "PayPal",
                                                   "account_number": merchant_email or "paypal-sandbox",
                                                   "currency": "USD", "gl_account": CLEARING_CODE,
                                                   "feed_provider": "Manual"})
        print(f"[{key}] bank account PayPal clearing")
    return bank["id"]


def ensure_crm(era, key):
    crm = era.must("GET", "/crm/settings")
    if not crm.get("enabled"):
        era.must("PUT", "/crm/settings", {"enabled": True})
        print(f"[{key}] CRM on")


def ensure_products(era, key, rows):
    have = {p["name"]: p for p in era.must("GET", "/products")}
    ids = {}
    for name, kind, uom, price, sales, purchase in rows:
        if name not in have:
            body = {"name": name, "product_type": kind, "uom": uom, "currency": "USD", "vat_treatment": "OutOfScope",
                    "track_inventory": False}
            if price is not None:
                body["unit_price"] = price
            if sales:
                body["sales_account"] = sales
            if purchase:
                body["purchase_account"] = purchase
            have[name] = {"id": era.must("POST", "/products", body)["id"]}
            print(f"[{key}] product {name}")
        ids[name] = have[name]["id"]
    return ids


def plan_products():
    rows = []
    for name, cents in PLANS:
        annual = round(cents * 12 * 0.8)
        rows.append((f"WebAgent {name} plan, monthly", "Service", "Month", f"{cents / 100:.2f}", "5100", None))
        rows.append((f"WebAgent {name} plan, 12 months", "Service", "Each", f"{annual / 100:.2f}", "5100", None))
    return rows


def notes(company=None, paypal=None, role=None):
    lines = []
    if company:
        lines.append(f"agentrix.company: {company}")
    if paypal:
        lines.append(f"agentrix.paypal: {paypal}")
    if role:
        lines.append(f"agentrix.role: {role}")
    return "\n".join(lines) or None


def ensure_party(era, key, kind, spec):
    path = "/customers" if kind == "customer" else "/vendors"
    have = {p["name"]: p for p in era.must("GET", path)}
    row = have.get(spec["name"])
    if row is None:
        body = {"name": spec["name"], "email": [], "phone": [], "currency": "USD",
                "payment_terms": spec.get("terms", "Net30"), "notes": spec.get("notes")}
        if spec.get("email"):
            body["email"] = [{"email": spec["email"], "label": None, "is_primary": True}]
        if kind == "customer" and spec.get("credit_limit"):
            body["credit_limit"] = spec["credit_limit"]
        if kind == "vendor":
            if spec.get("expense_account"):
                body["default_expense_account"] = spec["expense_account"]
            if spec.get("paypal"):
                body["bank_details"] = {"bank_name": "PayPal", "account_name": spec["name"], "account_number": spec["paypal"]}
        pid = era.must("POST", path, body)["id"]
        print(f"[{key}] {kind} {spec['name']}")
        return pid
    if kind == "customer" and spec.get("credit_limit"):
        current = row.get("credit_limit")
        if current is None or float(current) != float(spec["credit_limit"]):
            era.must("PUT", f"/customers/{row['id']}", {"credit_limit": float(spec["credit_limit"])})
            print(f"[{key}] credit limit for {spec['name']}: {spec['credit_limit']}")
    return row["id"]


def ensure_judge(era, key, spec, password):
    """The company's read-only login for judges: ERA's Viewer role, never more."""
    users = era.must("GET", "/users")
    rows = users if isinstance(users, list) else users.get("data", [])
    row = next((u for u in rows if str(u.get("email", "")).lower() == spec["judge"]), None)
    if row is None:
        era.must("POST", "/users", {"email": spec["judge"], "display_name": f"Judges (read-only), {spec['name']}",
                                    "role": JUDGE_ROLE, "password": password})
        print(f"[{key}] read-only login {spec['judge']} ({JUDGE_ROLE})")
    elif row.get("role") != JUDGE_ROLE or row.get("is_active") is False:
        era.must("PUT", f"/users/{row['id']}", {"role": JUDGE_ROLE, "is_active": True})
        print(f"[{key}] read-only login {spec['judge']} set back to {JUDGE_ROLE}")
    probe = Era(era.base[: -len("/api/v1")])
    status, payload = probe.call("POST", "/auth/login", {"email": spec["judge"], "password": password})
    if status != 200 or payload.get("user", {}).get("role") != JUDGE_ROLE:
        print(f"[{key}] WARNING: {spec['judge']} does not log in as {JUDGE_ROLE} with the configured password ({status})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:18180")
    ap.add_argument("--state", default=os.path.expanduser("~/.config/agentrix-saas/era"))
    ap.add_argument("--env", default=None)
    args = ap.parse_args()

    env = read_env(args.env)
    os.makedirs(args.state, mode=0o700, exist_ok=True)
    tenants_path = os.path.join(args.state, "tenants.json")
    tenants = json.load(open(tenants_path)) if os.path.exists(tenants_path) else {}
    for key, spec in COMPANIES.items():
        t = tenants.setdefault(key, {})
        t.setdefault("name", spec["name"])
        t.setdefault("email", spec["email"])
        t.setdefault("password", secrets.token_urlsafe(24))

    def save():
        fd = os.open(tenants_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(tenants, f, indent=2)
        os.chmod(tenants_path, 0o600)

    judges = tenants.setdefault("judges", {})
    judges["emails"] = {k: s["judge"] for k, s in COMPANIES.items()}
    judge_password = None  # filled once the .env is read

    save()  # the generated logins exist before any signup uses them

    if env.get("SAAS_ERA_DEMO_PASSWORD"):
        judge_password = env["SAAS_ERA_DEMO_PASSWORD"]
        judges.pop("password", None)
        judges["password_from"] = "SAAS_ERA_DEMO_PASSWORD"
    else:
        judge_password = judges.setdefault("password", secrets.token_urlsafe(12))
        judges["password_from"] = "tenants.json"
    save()

    era = Era(args.url)
    merchants = {k: env.get(s["merchant_env"], "") for k, s in COMPANIES.items()}
    partner = env.get("PAYPAL_PARTNER_EMAIL", "")
    summary = {}
    for key, spec in COMPANIES.items():
        creds = tenants[key]
        ensure_tenant(era, key, spec, creds)
        save()
        ensure_usd(era, key)
        bank_id = ensure_accounts(era, key, merchants[key])
        ensure_crm(era, key)
        rows = COMMON_PRODUCTS + (plan_products() if key == "agentrix" else KILELE_PRODUCTS)
        products = ensure_products(era, key, rows)

        other = "kilele-bakery" if key == "agentrix" else "agentrix"
        other_name = COMPANIES[other]["name"]
        customers = {
            other_name: {"name": other_name, "email": COMPANIES[other]["email"], "terms": "Net14",
                         "credit_limit": "800.00" if key == "agentrix" else "300.00",
                         "notes": notes(company=other, paypal=merchants[other])},
        }
        vendors = {
            other_name: {"name": other_name, "email": COMPANIES[other]["email"], "terms": "Net14",
                         "paypal": merchants[other], "expense_account": "7350" if key == "kilele-bakery" else "7900",
                         "notes": notes(company=other, paypal=merchants[other])},
            "Vultr": {"name": "Vultr", "email": "billing@vultr.com", "terms": "DueOnReceipt", "expense_account": "7350",
                      "notes": notes(role="cloud hosting and inference")},
            "Anthropic": {"name": "Anthropic", "email": "billing@anthropic.com", "terms": "DueOnReceipt",
                          "expense_account": "7350", "notes": notes(role="Claude model usage")},
            "Wanjiru Kamau (contractor)": {"name": "Wanjiru Kamau (contractor)", "email": "wanjiru.contractor@example.com",
                                           "terms": "Net7", "expense_account": "7500",
                                           "paypal": "wanjiru.contractor@example.com",
                                           "notes": notes(role="contractor")},
            "Savannah Growth Partners (referral)": {"name": "Savannah Growth Partners (referral)",
                                                    "email": partner or "referrals@example.com", "terms": "Net30",
                                                    "expense_account": "7700", "paypal": partner or None,
                                                    "notes": notes(paypal=partner or None, role="referral partner")},
        }
        ensure_judge(era, key, spec, judge_password)
        cust_ids = {n: ensure_party(era, key, "customer", s) for n, s in customers.items()}
        vend_ids = {n: ensure_party(era, key, "vendor", s) for n, s in vendors.items()}
        summary[key] = {"entity_id": creds["entity_id"], "judge_login": spec["judge"],
                        "paypal_clearing_bank_account": bank_id,
                        "customers": cust_ids, "vendors": vend_ids, "products": len(products)}

    # ERA's customer portal (where CRM tickets come from) serves the tenant named by ENTITY_ID.
    entity_file = os.path.join(args.state, "entity_id")
    current = open(entity_file).read().strip() if os.path.exists(entity_file) else ""
    wanted = tenants["agentrix"]["entity_id"]
    if current != wanted:
        with open(entity_file, "w") as f:
            f.write(wanted + "\n")
        print("ENTITY_ID now Agentrix Ltd's tenant: run `era.sh restart` so the support portal serves it")

    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
