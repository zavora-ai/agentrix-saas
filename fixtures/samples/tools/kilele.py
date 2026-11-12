#!/usr/bin/env python3
"""Writes the Kilele Bakery sample bundle (specs/platform/03-data.md §5.3, 01a §1.4, D1).

Run from the repository root: python3 fixtures/samples/tools/kilele.py
Every person, phone and email is fictional (phones in Ofcom's drama range, emails at
example.com). Times are relative to the load instant (agentrix-records/src/time.rs).
Totals are never written here: the loader sums lines, products and zones.
"""
import json
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "kilele-bakery")


def write(name, rows):
    path = os.path.join(OUT, "records", f"{name}.jsonl")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


manifest = {
    "site": "kilele-bakery-sample",
    "host": "kilele-bakery-sample.agentrix.website",
    "name": "Kilele Bakery",
    "archetype": "bakery",
    "currency": "KES",
    "utc_offset_min": 180,
    "seed_version": "2026-11-08.7",
    "facts": {
        "order_prefix": "KB",
        "domain": "kilele-bakery.example",
        "since": "2016",
        "headline": "Baked at dawn\nin Westlands.",
        "intro": "Sourdough, croissants and celebration cakes since 2016. Open from 6:30 Monday to Saturday and 7:00 on Sunday. Weekend delivery to Westlands and Kilimani.",
        "owner": {"name": "Wanjiru Kamau", "first": "Wanjiru", "initial": "W", "channel": "WhatsApp", "wait": "~2 h", "reply": "usually within 2 hours (7:00–20:00)", "login": {"email": "wanjiru@kilele-bakery.example", "password": "kilele-sample-owner"}},
        "address": "Woodvale Grove, Westlands, Nairobi",
        "parking": "Parking at the back, off Woodvale Grove",
        "hours": [
            {"days": "Mon–Sat", "open": "06:30", "close": "19:00", "weekdays": [1, 2, 3, 4, 5, 6]},
            {"days": "Sun", "open": "07:00", "close": "14:00", "weekdays": [0]},
        ],
        "collection": "Collection slots: Mon–Sat 08:00–12:00 and 14:00–16:00; Sun 08:00–12:00.",
        "confirmations": "Wanjiru confirms every cake order herself on WhatsApp, usually within 2 hours (7:00–20:00).",
        "changes": "Changes and cancellations are free until 24 hours before your slot.",
        "changes_cutoff_hours": 24,
        "hold_grant": {"id": "g-2", "max": 3, "until": "12:00", "words": "The Bread desk can hold up to 3 loaves for you until 12:00, free. You pay at the counter."},
        "refund_rule": {"id": "g-3", "per_order": 2000, "per_week": 5, "until": "31 Dec 2026", "to": "the original payment method"},
        "allergens": "Kilele's kitchen uses wheat flour, so it can't promise anything is safe for coeliac guests.",
        "not_offered": ["Sculpted figures", "Printed photos on cakes", "Any cake sooner than its notice"],
        "release": "v14",
        "classes": "Saturday baking class, 9:00–13:00, eight seats, lunch included.",
        "mark": {"family": "grain", "squareness": 4.6, "key": "kilele", "font": "Fraunces, Georgia, serif"},
        "front_color": "#3b2416",
        "desks": [
            {"id": "bread", "name": "Bread", "href": "/bread", "color": "#9c5725", "glyph": "loaf", "seed": 3,
             "mandate": "I know what came out of the oven today, what's left and when the next batch is due.",
             "starters": [["What's still warm?", "stock", True], ["Anything without dairy?", "dairy", False], ["Can you hold 2 loaves for me?", "hold", False]]},
            {"id": "cakes", "name": "Cakes", "href": "/cakes", "color": "#8e3b46", "glyph": "cake", "seed": 7,
             "mandate": "I help you design a cake from Kilele's own options. Prices come from the price list. Wanjiru confirms every cake.",
             "starters": [["Design a birthday cake with me", "design", False], ["How much for 30 people?", "serves", False], ["Is anything gluten-free?", "gluten", False]]},
            {"id": "orders", "name": "Orders", "href": "/orders", "color": "#3b2416", "glyph": "bag", "seed": 11,
             "mandate": "I find your order and say where it stands. Changes go to Wanjiru.",
             "starters": [["Where is my order?", "where", False], ["Change my delivery time", "change", False], ["Can I cancel?", "cancel", False]]},
            {"id": "visit", "name": "Visit", "href": "/visit", "color": "#ad8b22", "glyph": "pin", "seed": 5,
             "mandate": "I know the hours, directions, parking and where Kilele delivers.",
             "starters": [["Are you open on Sunday?", "hours", True], ["Do you deliver to Lavington?", "deliver", False], ["Where do I park?", "park", True]]},
        ],
    },
}
os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "manifest.json"), "w") as f:
    json.dump(manifest, f, ensure_ascii=False, indent=2)
    f.write("\n")

products = [
    ("sourdough", "Country sourdough", 450, "bread", "Our everyday loaf, shaped the night before and baked at dawn.", {"ingredients": "Flour, water, salt.", "contains": ["wheat"], "tone": "crust"}),
    ("rye", "Rye and caraway", 500, "bread", "A dense rye loaf with caraway seed.", {"ingredients": "Rye and wheat flour, water, salt, caraway.", "contains": ["wheat", "rye"], "tone": "rye"}),
    ("croissant", "Butter croissant", 180, "bread", "Laminated by hand, two batches a morning.", {"ingredients": "Contains butter, milk, egg.", "contains": ["wheat", "milk", "egg"], "tone": "golden"}),
    ("cardamom-bun", "Cardamom bun", 150, "bread", "Soft knotted bun with ground cardamom and sugar.", {"ingredients": "Contains butter, milk.", "contains": ["wheat", "milk"], "tone": "golden"}),
    ("mandazi", "Mandazi, six", 200, "bread", "Six coconut mandazi, fried fresh each morning.", {"ingredients": "Contains coconut milk.", "contains": ["wheat", "coconut"], "tone": "golden"}),
    ("banana-bread", "Banana bread slice", 220, "bread", "A thick slice of banana loaf.", {"ingredients": "Contains butter, egg.", "contains": ["wheat", "milk", "egg"], "tone": "rye"}),
    ("cinnamon-roll", "Cinnamon roll", 220, "bread", "Rolled with brown sugar and cinnamon.", {"ingredients": "Contains butter, milk.", "contains": ["wheat", "milk"], "tone": "dark"}),
    ("red-velvet", "Red velvet slice", 350, "bread", "Red velvet sponge with cream cheese frosting.", {"ingredients": "Contains butter, milk, egg.", "contains": ["wheat", "milk", "egg"], "tone": "velvet"}),
    ("weekend-bake-box", "Weekend bake box", 1350, "box", "A sourdough loaf, four cinnamon rolls and a banana bread loaf, baked on Saturday morning.", {"ingredients": "Contains butter, milk, egg.", "contains": ["wheat", "milk", "egg"], "days": ["sat"], "tone": "crust"}),
    ("fruit-cake", "Christmas fruit cake", 2800, "seasonal", "Rich fruit cake fed with orange syrup, 1 kg. Collection 19–23 Dec.", {"ingredients": "Contains butter, egg, nuts.", "contains": ["wheat", "milk", "egg", "nuts"], "tone": "dark"}),
]
write("products", [
    {"sku": sku, "slug": sku, "section": sec, "name": name, "price": price, "description": desc, "attrs": attrs, "sort": i}
    for i, (sku, name, price, sec, desc, attrs) in enumerate(products)
])

# Today's bake: what is on the shelf from earlier batches, and later batches by the clock.
write("bake_plan", [
    {"sku": "sourdough", "planned": 8, "batches": ["06:30"]},
    {"sku": "rye", "planned": 10, "batches": ["06:30"]},
    {"sku": "croissant", "planned": 2, "batches": ["06:30", "10:00"]},
    {"sku": "cardamom-bun", "planned": 14, "batches": ["06:30"]},
    {"sku": "mandazi", "planned": 8, "batches": ["06:30"]},
    {"sku": "banana-bread", "planned": 9, "batches": ["06:30"]},
    {"sku": "cinnamon-roll", "planned": 0, "batches": ["06:30"]},
    {"sku": "red-velvet", "planned": 12, "batches": ["06:30"]},
    {"sku": "weekend-bake-box", "planned": 5, "batches": ["06:30"]},
    {"sku": "fruit-cake", "planned": 60, "batches": ["06:30"]},
])

write("price_lists", [
    {"version": "v14", "status": "live", "usd_rate": 129.00, "valid_from": "d-6 09:00", "valid_until": "sat+1 08:00", "source": "Confirmed by Wanjiru"},
    {"version": "v15", "status": "draft", "usd_rate": 129.00, "valid_from": "d+3 09:00", "valid_until": "sat+3 08:00", "source": "The Editor's draft from Wanjiru's WhatsApp list, waiting for her"},
])
rules = [
    ("size.1", "size", "1 tier", 3800, {"serves": "12–15", "notice_days": 2}),
    ("size.2", "size", "2 tiers", 4500, {"serves": "25–30", "notice_days": 2}),
    ("size.3", "size", "3 tiers", 6900, {"serves": "40–45", "notice_days": 3}),
    ("sponge.vanilla", "sponge", "Vanilla sponge", 0, {}),
    ("sponge.lemon", "sponge", "Lemon sponge", 0, {}),
    ("sponge.chocolate", "sponge", "Chocolate sponge", 200, {}),
    ("sponge.redvelvet", "sponge", "Red velvet sponge", 300, {}),
    ("sponge.gf", "sponge", "Gluten-free sponge", 600, {"only_sizes": ["size.1", "size.2"], "note": "Kilele's kitchen uses wheat flour, so it can't promise anything is safe for coeliac guests."}),
    ("colour.cream", "colour", "Cream buttercream", 0, {"swatch": "#f3e4c8"}),
    ("colour.blush", "colour", "Blush pink buttercream", 0, {"swatch": "#f0c3c4"}),
    ("colour.forest", "colour", "Forest green buttercream", 0, {"swatch": "#2f5d3a"}),
    ("colour.chocolate", "colour", "Chocolate buttercream", 0, {"swatch": "#5a3825"}),
    ("colour.sky", "colour", "Sky blue buttercream", 0, {"swatch": "#a9cbe6"}),
    ("finish.smooth", "finish", "Smooth finish", 0, {}),
    ("finish.naked", "finish", "Naked finish", 0, {}),
    ("finish.ombre", "finish", "Ombré finish", 250, {}),
    ("finish.drip", "finish", "Chocolate drip", 400, {}),
    ("deco.none", "deco", "No decoration", 0, {}),
    ("deco.berries", "deco", "Fresh berries", 500, {}),
    ("deco.flowers", "deco", "Edible flowers", 700, {}),
    ("deco.sprinkles", "deco", "Sprinkles", 150, {}),
    ("message", "message", "Message on the cake", 0, {"max_chars": 22}),
]
rows = []
for version, changes in (("v14", {}), ("v15", {"size.1": 4000, "size.2": 4800})):
    for i, (code, grp, label, amount, meta) in enumerate(rules):
        rows.append({"version": version, "code": code, "grp": grp, "label": label, "amount": changes.get(code, amount), "meta": meta, "sort": i})
write("price_rules", rows)

write("zones", [
    {"zone": "zone1", "label": "Westlands and Kilimani", "fee": 300, "areas": ["Westlands", "Kilimani"], "note": "Orders in the same slot share one delivery fee."},
])
write("slot_rules", [
    {"kind": "delivery", "weekdays": ["sat"], "start": "10:00", "end": "12:00", "capacity": 6, "weeks": 7},
    {"kind": "delivery", "weekdays": ["sat"], "start": "12:00", "end": "14:00", "capacity": 6, "weeks": 7},
    {"kind": "delivery", "weekdays": ["sun"], "start": "10:00", "end": "12:00", "capacity": 4, "weeks": 7},
    {"kind": "collection", "weekdays": ["mon", "tue", "wed", "thu", "fri", "sat"], "start": "08:00", "end": "10:00", "capacity": 12, "weeks": 7},
    {"kind": "collection", "weekdays": ["mon", "tue", "wed", "thu", "fri", "sat"], "start": "10:00", "end": "12:00", "capacity": 12, "weeks": 7},
    {"kind": "collection", "weekdays": ["mon", "tue", "wed", "thu", "fri", "sat"], "start": "14:00", "end": "16:00", "capacity": 12, "weeks": 7},
    {"kind": "collection", "weekdays": ["sun"], "start": "08:00", "end": "10:00", "capacity": 8, "weeks": 7},
    {"kind": "collection", "weekdays": ["sun"], "start": "10:00", "end": "12:00", "capacity": 8, "weeks": 7},
])

sample_names = [
    "Wairimu N.", "Otieno K.", "Chebet R.", "Kamau J.", "Nyambura W.", "Odhiambo P.", "Wambua M.", "Jeptoo L.",
    "Mwende S.", "Kiprono T.", "Nduta G.", "Omondi B.", "Wanjala E.", "Akinyi F.", "Mutua D.", "Kerubo H.",
    "Gitau A.", "Moraa C.", "Kiplagat N.", "Wangari V.", "Barasa I.", "Nekesa O.", "Karanja Y.", "Auma Z.",
    "Kimani Q.", "Jelagat U.", "Ouma X.", "Wanjiku T.", "Rotich S.", "Nafula R.", "Njoroge M.",
]
customers = [
    {"customer_id": "cus-njeri", "name": "Njeri", "email": "njeri@example.com", "phone": "+44 7700 900418", "place": "London", "created_at": "d-40 19:02"},
    {"customer_id": "cus-amani", "name": "Amani W.", "email": "amani.w@example.com", "phone": "+44 7700 900431", "place": "Nairobi", "created_at": "d-21 08:10"},
    {"customer_id": "cus-mary", "name": "Mary K.", "email": "mary.k@example.com", "phone": "+44 7700 900452", "place": "Nairobi", "created_at": "d-3 18:40"},
    {"customer_id": "cus-peter", "name": "Peter O.", "email": "peter.o@example.com", "phone": "+44 7700 900467", "place": "Nairobi", "created_at": "d-30 12:00"},
    {"customer_id": "cus-riverside", "name": "Riverside Co.", "email": "orders@riverside.example", "kind": "business", "place": "Nairobi", "created_at": "d-2 10:15"},
]
# Regulars who ordered this week (fictional; phones in Ofcom's drama range).
for cid, name, email, phone, place, at in [
    ("cus-leah", "Leah M.", "leah.m@example.com", "+44 7700 900503", "Westlands", "d-15 09:30"),
    ("cus-brian", "Brian N.", "brian.n@example.com", "+44 7700 900516", "Kilimani", "d-9 13:05"),
    ("cus-faith", "Faith W.", "faith.w@example.com", "+44 7700 900527", "Westlands", "d-6 18:20"),
    ("cus-kevin", "Kevin O.", "kevin.o@example.com", "+44 7700 900538", "Kilimani", "d-5 07:45"),
    ("cus-esther", "Esther A.", "esther.a@example.com", "+44 7700 900549", "Westlands", "d-2 11:10"),
]:
    customers.append({"customer_id": cid, "name": name, "email": email, "phone": phone, "place": place, "created_at": at})
for i, name in enumerate(sample_names, start=1):
    customers.append({"customer_id": f"cus-s{i:02d}", "name": f"{name} (Sample customer)", "email": f"sample{i:02d}@example.com", "place": "Nairobi", "created_at": f"d-{60 - i} 09:00"})
write("customers", customers)

cake_1043 = {"size": "size.2", "sponge": "sponge.lemon", "colour": "colour.forest", "finish": "finish.ombre", "deco": "deco.berries", "message": "Happy 7th Amina", "note": "No nuts", "date": None}
cake_1044 = {"size": "size.2", "sponge": "sponge.vanilla", "colour": "colour.cream", "finish": "finish.smooth", "deco": "deco.none", "message": "Happy 30th Amani", "note": "", "date": None}
orders = [
    {"order_id": "KB-1038", "customer": "cus-peter", "contact_name": "Peter O.", "contact_email": "peter.o@example.com", "kind": "catalogue", "state": "collected",
     "fulfilment": "collection", "slot": "wed-1 08:00", "slot_kind": "collection", "price_list": "v14", "created_at": "d-6 19:30",
     "lines": [{"sku": "red-velvet", "qty": 4}, {"sku": "croissant", "qty": 2}],
     "events": [{"step": "requested", "at": "d-6 19:30", "actor": "Peter O.", "note": "Order placed"},
                {"step": "paid", "at": "d-6 19:32", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."},
                {"step": "ready", "at": "wed-1 07:40", "actor": "Wanjiru", "note": "Ready to collect."},
                {"step": "collected", "at": "wed-1 08:20", "actor": "Wanjiru", "note": "Collected."}],
     "payments": [{"payment_id": "pay-s1038", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-6 19:32"}]},
    {"order_id": "KB-1042", "customer": "cus-njeri", "contact_name": "Njeri", "contact_email": "njeri@example.com", "contact_phone": "+44 7700 900418",
     "recipient": "Amina, at home in Kilimani", "kind": "catalogue", "state": "delivered", "fulfilment": "delivery", "zone": "zone1",
     "slot": "sat-1 10:00", "slot_kind": "delivery", "price_list": "v14", "created_at": "d-10 20:14",
     "lines": [{"sku": "sourdough", "qty": 2}, {"sku": "weekend-bake-box", "qty": 1}],
     "events": [{"step": "requested", "at": "d-10 20:14", "actor": "Njeri", "note": "Order placed"},
                {"step": "paid", "at": "d-10 20:16", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."},
                {"step": "baking", "at": "sat-1 06:00", "actor": "Wanjiru", "note": "Baking now."},
                {"step": "out", "at": "sat-1 09:40", "actor": "Wanjiru", "note": "Out for delivery."},
                {"step": "delivered", "at": "sat-1 10:52", "actor": "Wanjiru", "note": "Delivered to Amina."}],
     "payments": [{"payment_id": "pay-s1042", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-10 20:16"}]},
    {"order_id": "KB-1043", "customer": "cus-njeri", "contact_name": "Njeri", "contact_email": "njeri@example.com", "contact_phone": "+44 7700 900418",
     "kind": "custom", "state": "paid", "fulfilment": "collection",
     "slot": "sat+1 10:00", "slot_kind": "collection", "price_list": "v14", "note": "No nuts", "spec": cake_1043, "created_at": "d-1 21:10",
     "lines": [{"rule": "size.2"}, {"rule": "sponge.lemon"}, {"rule": "colour.forest"}, {"rule": "finish.ombre"}, {"rule": "deco.berries"}],
     "events": [{"step": "requested", "at": "d-1 21:10", "actor": "Njeri", "note": "Custom cake requested. Held for Wanjiru; nothing charged."},
                {"step": "approved", "at": "d-1 21:48", "actor": "Wanjiru", "note": "Approved with her passkey. The payment request is on the order page."},
                {"step": "paid", "at": "d-1 22:05", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."}],
     "payments": [{"payment_id": "pay-s1043", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-1 22:05"}]},
    {"order_id": "KB-1044", "customer": "cus-amani", "contact_name": "Amani W.", "contact_email": "amani.w@example.com", "contact_phone": "+44 7700 900431",
     "kind": "custom", "state": "held", "fulfilment": "collection", "slot": "sat+1 10:00", "slot_kind": "collection", "price_list": "v14",
     "spec": cake_1044, "created_at": "m-95",
     "lines": [{"rule": "size.2"}, {"rule": "sponge.vanilla"}, {"rule": "colour.cream"}, {"rule": "finish.smooth"}, {"rule": "deco.none"}],
     "events": [{"step": "requested", "at": "m-95", "actor": "Amani W.", "note": "Custom cake requested. Held for Wanjiru; nothing charged."}]},
]
# 23 Christmas preorders: 11 in the first Saturday collection window, the rest across the week.
slots = ["sat+6 10:00"] * 11 + ["sun+6 10:00"] * 4 + ["mon+7 10:00"] * 4 + ["tue+7 10:00"] * 4
for i, slot in enumerate(slots, start=1):
    orders.append({
        "order_id": f"KB-{1000 + i:04d}", "customer": f"cus-s{i:02d}", "contact_name": sample_names[i - 1], "contact_email": f"sample{i:02d}@example.com",
        "kind": "catalogue", "state": "paid", "fulfilment": "collection", "slot": slot, "slot_kind": "collection", "price_list": "v14",
        "created_at": f"d-{24 - i} 1{i % 10}:15", "lines": [{"sku": "fruit-cake", "qty": 1}],
        "events": [{"step": "requested", "at": f"d-{24 - i} 1{i % 10}:15", "actor": sample_names[i - 1], "note": "Christmas preorder placed"},
                   {"step": "paid", "at": f"d-{24 - i} 1{i % 10}:17", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."}],
        "payments": [{"payment_id": f"pay-s{1000 + i}", "rail": "paypal", "currency": "USD", "state": "sample", "at": f"d-{24 - i} 1{i % 10}:17"}],
    })
def custom(oid, cid, name, email, phone, cake, state, slot, kind, created, events, zone=None):
    lines = [{"rule": cake[g]} for g in ("size", "sponge", "colour", "finish", "deco")]
    o = {"order_id": oid, "customer": cid, "contact_name": name, "contact_email": email, "contact_phone": phone,
         "kind": "custom", "state": state, "fulfilment": "delivery" if zone else "collection", "slot": slot, "slot_kind": kind,
         "price_list": "v14", "spec": cake, "created_at": created, "lines": lines, "events": events}
    if zone:
        o["zone"] = zone
    if cake.get("note"):
        o["note"] = cake["note"]
    return o


HELD = "Custom cake requested. Held for Wanjiru; nothing charged."
orders += [
    custom("KB-1045", "cus-leah", "Leah M.", "leah.m@example.com", "+44 7700 900503",
           {"size": "size.1", "sponge": "sponge.chocolate", "colour": "colour.blush", "finish": "finish.drip", "deco": "deco.sprinkles", "message": "Happy 5th Zuri", "note": "", "date": None},
           "held", "wed+1 10:00", "collection", "m-40",
           [{"step": "requested", "at": "m-40", "actor": "Leah M.", "note": HELD}]),
    custom("KB-1046", "cus-brian", "Brian N.", "brian.n@example.com", "+44 7700 900516",
           {"size": "size.3", "sponge": "sponge.redvelvet", "colour": "colour.cream", "finish": "finish.smooth", "deco": "deco.flowers", "message": "Congratulations Imani", "note": "Graduation party, about 40 guests", "date": None},
           "held", "sat+2 10:00", "delivery", "m-180",
           [{"step": "requested", "at": "m-180", "actor": "Brian N.", "note": HELD}], zone="zone1"),
    custom("KB-1047", "cus-faith", "Faith W.", "faith.w@example.com", "+44 7700 900527",
           {"size": "size.2", "sponge": "sponge.vanilla", "colour": "colour.sky", "finish": "finish.naked", "deco": "deco.berries", "message": "Welcome home Tumi", "note": "", "date": None},
           "approved", "sun+1 10:00", "collection", "d-1 18:00",
           [{"step": "requested", "at": "d-1 18:00", "actor": "Faith W.", "note": HELD},
            {"step": "approved", "at": "d-1 19:10", "actor": "Wanjiru", "note": "Approved with her passkey. The payment request is on the order page."}]),
    custom("KB-1048", "cus-kevin", "Kevin O.", "kevin.o@example.com", "+44 7700 900538",
           {"size": "size.1", "sponge": "sponge.lemon", "colour": "colour.forest", "finish": "finish.ombre", "deco": "deco.none", "message": "", "note": "", "date": None},
           "declined", "sat+1 10:00", "collection", "d-2 09:15",
           [{"step": "requested", "at": "d-2 09:15", "actor": "Kevin O.", "note": HELD},
            {"step": "declined", "at": "d-2 10:02", "actor": "Wanjiru", "note": "Saturday morning is full for cakes. Sunday works if you'd like to ask again. Nothing was charged."}]),
    {"order_id": "KB-1049", "customer": "cus-esther", "contact_name": "Esther A.", "contact_email": "esther.a@example.com", "kind": "catalogue", "state": "paid",
     "fulfilment": "collection", "slot": "mon+1 10:00", "slot_kind": "collection", "price_list": "v14", "created_at": "m-75",
     "lines": [{"sku": "croissant", "qty": 3}, {"sku": "cardamom-bun", "qty": 2}],
     "events": [{"step": "requested", "at": "m-75", "actor": "Esther A.", "note": "Order placed"},
                {"step": "paid", "at": "m-73", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."}],
     "payments": [{"payment_id": "pay-s1049", "rail": "paypal", "currency": "USD", "state": "sample", "at": "m-73"}]},
    {"order_id": "KB-1050", "customer": "cus-leah", "contact_name": "Leah M.", "contact_email": "leah.m@example.com", "kind": "catalogue", "state": "paid",
     "fulfilment": "delivery", "zone": "zone1", "slot": "sat+1 12:00", "slot_kind": "delivery", "price_list": "v14", "created_at": "d-1 07:20",
     "recipient": "Leah M., at home in Westlands",
     "lines": [{"sku": "weekend-bake-box", "qty": 1}, {"sku": "rye", "qty": 1}],
     "events": [{"step": "requested", "at": "d-1 07:20", "actor": "Leah M.", "note": "Order placed"},
                {"step": "paid", "at": "d-1 07:22", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."}],
     "payments": [{"payment_id": "pay-s1050", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-1 07:22"}]},
    {"order_id": "KB-1051", "customer": "cus-kevin", "contact_name": "Kevin O.", "contact_email": "kevin.o@example.com", "kind": "catalogue", "state": "paid",
     "fulfilment": "collection", "slot": "sun+1 08:00", "slot_kind": "collection", "price_list": "v14", "created_at": "d-1 21:40",
     "lines": [{"sku": "sourdough", "qty": 1}, {"sku": "mandazi", "qty": 2}],
     "events": [{"step": "requested", "at": "d-1 21:40", "actor": "Kevin O.", "note": "Order placed"},
                {"step": "paid", "at": "d-1 21:42", "actor": "the Cashier", "note": "Paid with PayPal at Kilele's rate (sample, no money moved)."}],
     "payments": [{"payment_id": "pay-s1051", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-1 21:42"}]},
]
write("orders", orders)
write("refunds", [
    {"refund_id": "R-1042-1", "order_id": "KB-1042", "sku": "weekend-bake-box", "qty": 1, "reason": "The banana bread loaf was missing from the bake box.", "rule": "g-3", "state": "done", "at": "sat-1 13:20"},
    {"refund_id": "R-1038-1", "order_id": "KB-1038", "amount": 800, "reason": "Two slices arrived crushed.", "rule": "g-3", "state": "done", "at": "wed-1 11:05"},
])
write("holds", [
    {"hold_id": "H-0192", "sku": "rye", "qty": 1, "until": "d+0 12:00", "name": "Faith W.", "grant_id": "g-2", "state": "held", "created_at": "m-55"},
    {"hold_id": "H-0193", "sku": "sourdough", "qty": 2, "until": "d+0 12:00", "name": "Njeri", "grant_id": "g-2", "state": "held", "created_at": "m-30"},
])
write("bookings", [
    {"booking_id": "BK-0071", "service": "Saturday baking class", "starts_at": "sat+1 09:00", "customer": "cus-peter", "name": "Peter O.", "seats": 1, "created_at": "d-4 16:20"},
    {"booking_id": "BK-0072", "service": "Saturday baking class", "starts_at": "sat+1 09:00", "customer": "cus-leah", "name": "Leah M.", "seats": 1, "created_at": "d-3 08:05"},
    {"booking_id": "BK-0073", "service": "Saturday baking class", "starts_at": "sat+1 09:00", "customer": "cus-esther", "name": "Esther A.", "seats": 2, "created_at": "d-1 12:40"},
])
write("questions", [
    {"question_id": "q-s1", "desk": "Cakes", "question": "Wedding cake for 120 guests on 9 Jan, three tiers, gluten-free. Can you do it?", "contact": "mary.k@example.com", "page": "/cakes", "at": "d-3 18:42"},
    {"question_id": "q-s2", "desk": "Orders", "question": "Can you quote 40 breakfast boxes for our team on Fridays?", "contact": "orders@riverside.example", "page": "/orders", "at": "d-2 10:16"},
    {"question_id": "q-s3", "desk": "Cakes", "question": "Birthday cake shaped like a football for Saturday?", "contact": "+44 7700 900488", "page": "/cakes/studio", "at": "d-1 12:30"},
    {"question_id": "q-s4", "desk": "Visit", "question": "Do you deliver to Lavington?", "page": "/visit", "at": "m-200"},
    {"question_id": "q-s5", "desk": "Cakes", "question": "Can you make a cake with no eggs for my son's birthday?", "contact": "faith.w@example.com", "page": "/cakes", "at": "m-150"},
    {"question_id": "q-s6", "desk": "Bread", "question": "Do you sell some of your sourdough starter?", "page": "/bread", "at": "m-95"},
])
print("wrote", os.path.normpath(OUT))
