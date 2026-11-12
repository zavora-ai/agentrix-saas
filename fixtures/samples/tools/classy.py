#!/usr/bin/env python3
"""Writes the Classy Perfumes sample bundle (specs/platform/03-data.md §5.4, 01a §1.5, D5, D10).

Run from the repository root: python3 fixtures/samples/tools/classy.py
Classy Perfumes is a real shop at classyperfumes.shop. This sample uses none of its products or
data: the catalogue is the concept's four invented scents plus invented ones, on its own
-sample host, marked Sample. Every person, phone and email is fictional (phones in Ofcom's drama
range, emails at example.com). Times are relative to the load instant
(agentrix-records/src/time.rs). Totals are never written here: the loader sums lines, products
and zones.
"""
import json
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "classy-perfumes")


def write(name, rows):
    path = os.path.join(OUT, "records", f"{name}.jsonl")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


journal = [
    {
        "slug": "how-amber-wears-in-cold-air",
        "title": "How amber wears in cold air",
        "at": "d-9 08:00",
        "standfirst": "Why Amber Noir's smoked vanilla opens slowly on a cool Nairobi evening.",
        "body": [
            "Amber isn't one material. In perfume it means a warm accord, usually labdanum resin, benzoin and vanilla.",
            "Cold air slows a perfume down. On a July evening in Nairobi, Amber Noir opens with the smoke first and lets the vanilla through later, once your skin warms it.",
            "Spray it on skin, not on a coat. Wool holds the smoke and loses the vanilla.",
            "If amber is new to you, start with a free 2 ml sample. Amber Lantern is the softer one, with cardamom where Amber Noir has smoke.",
        ],
    },
    {
        "slug": "scents-for-the-short-rains",
        "title": "Scents for the short rains",
        "at": "d-3 08:00",
        "standfirst": "Rain Oud and the petrichor accord behind this season's limited run.",
        "body": [
            "Petrichor is the smell of the first rain on dry ground. Perfumers rebuild it from a few materials that smell of wet stone and earth.",
            "Rain Oud puts that accord over oud, so it starts like a wet afternoon and dries down darker. It's a limited run for the short rains.",
            "For something lighter in the wet months, Karura Moss smells of oakmoss and wet leaves, and Lime Market keeps a bright edge when the sky is grey.",
            "Not sure which way to go? Add a free 2 ml sample of each to your next order and try them at home.",
        ],
    },
]

TOWNS = ["Mombasa", "Kisumu", "Nakuru", "Eldoret", "Nyeri", "Thika", "Machakos", "Malindi", "Kilifi", "Naivasha", "Nanyuki", "Kakamega", "Meru", "Kitale", "Embu", "Kisii", "Diani", "Lamu"]

manifest = {
    "site": "classy-perfumes-sample",
    "host": "classy-perfumes-sample.agentrix.website",
    "name": "Classy Perfumes",
    "archetype": "shop",
    "currency": "KES",
    "utc_offset_min": 180,
    "seed_version": "2026-11-08.7",
    "facts": {
        "order_prefix": "CP",
        "domain": "classy-perfumes.example",
        "headline": "Perfume and gift sets,\nwrapped free in Westlands.",
        "intro": f"Perfumes in three sizes across five scent families, gift sets wrapped by hand, same-day delivery in Nairobi and courier delivery to {len(TOWNS)} towns. Every name and price here is a sample.",
        "owner": {"name": "Makena Wafula", "first": "Makena", "initial": "M", "channel": "WhatsApp", "wait": "~3 hours", "hours": "9:00–19:00", "reply": "usually within 3 hours (9:00–19:00)", "login": {"email": "makena@classy-perfumes.example", "password": "classy-sample-owner"}},
        "address": "Westlands, Nairobi",
        "release": "v31",
        "same_day_cutoff": "14:00",
        "returns": {
            "days": 14,
            "opened": "Opened bottles can't be returned, so ask for a free 2 ml sample first.",
            "how": "Tell the Orders desk your order number and it passes the return to Makena, who replies with a courier pickup.",
        },
        "samples": "Any order can include a free 2 ml sample of any scent. Add it from a product page.",
        "wrapping": "Every order can be gift-wrapped free, with a handwritten card. Write your card message in the note for Makena at checkout.",
        "wrapping_how": "Makena wraps each gift in plum paper with a gold ribbon and writes your card by hand, word for word.",
        "not_published": [
            {"label": "Long-lasting", "terms": ["longevity", "long-lasting", "long lasting", "lasting", "last long", "lasts"], "what": "how long {x} lasts"},
            {"label": "Vegan", "terms": ["vegan", "cruelty", "animal"], "what": "whether {x} is vegan"},
        ],
        "families": [
            {"name": "Fresh citrus", "words": "Light and clean. For mornings and hot afternoons."},
            {"name": "Floral", "words": "Flowers, from soft violet to strong tuberose."},
            {"name": "Woody", "words": "Cedar, moss, vetiver and oud. Dry and calm."},
            {"name": "Amber", "words": "Warm resins and spice. For evenings."},
            {"name": "Gourmand", "words": "Vanilla, coffee and cocoa. Sweet and close to the skin."},
        ],
        "featured": ["vanilla-gift-set", "citrus-dawn", "amber-noir", "rain-oud"],
        "gift_bands": [5000, 7000],
        "example_plan": {
            "title": "Bridesmaids' gifts for Sat 12 Dec",
            "budget": 25000,
            "by": "Achieng",
            "people": "Wambui: fresh, sporty\nNyokabi: loves vanilla\nAwino: evenings out\nKawira: not sure\nJemutai: hates strong perfume",
        },
        "journal": journal,
        "mark": {"family": "facets", "squareness": 6.2, "key": "classy", "font": "Cormorant Garamond, Georgia, serif"},
        "front_color": "#1c1719",
        "desks": [
            {"id": "fragrances", "name": "Fragrances", "href": "/fragrances", "color": "#2a1f3d", "glyph": "perfume", "seed": 4,
             "mandate": "I explain notes, sizes and what is in stock, from this shop's catalogue.",
             "starters": [["What smells like rain?", "notes", False], ["Which is lightest?", "lightest", True], ["Is Citrus Dawn in stock?", "stock", True]]},
            {"id": "gifts", "name": "Gifts", "href": "/gifts/plan", "color": "#6d2e4f", "glyph": "gift", "seed": 19,
             "mandate": "I help you choose a gift from the catalogue and have it wrapped.",
             "starters": [["A gift under KSh 5,000", "budget", True], ["Plan gifts for a group", "plan", False], ["Is gift wrapping free?", "wrap", True]]},
            {"id": "orders", "name": "Orders", "href": "/orders", "color": "#3d5a6c", "glyph": "bag", "seed": 8,
             "mandate": "I find your order, tell you where it stands, and answer delivery and returns questions.",
             "starters": [["Where is my order?", "where", False], ["Do you deliver to Mombasa?", "deliver", True], ["Can I return it?", "returns", True]]},
            {"id": "journal", "name": "Journal", "href": "/journal", "color": "#5e5468", "glyph": "book", "seed": 27,
             "mandate": "I answer from the shop's journal posts only.",
             "starters": [["Which scent for the short rains?", "rains", False], ["How does amber wear in cold air?", "amber", False]]},
        ],
    },
}
os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "manifest.json"), "w") as f:
    json.dump(manifest, f, ensure_ascii=False, indent=2)
    f.write("\n")

# sku, name, section, size, price, family, notes, description, vanilla 0-3, strength 1-3, tone, stock, extra attrs
P = [
    # The concept's four (search.html:229-234).
    ("vanilla-gift-set", "Vanilla Gift Set", "gift", "30 ml", 4500, "Gourmand", "Vanilla, tonka, soft musk",
     "A vanilla candle and a 30 ml travel spray, gift-wrapped with a card.", 3, 1, "vanilla", 7, {"image": "classy-vanilla.jpg"}),
    ("citrus-dawn", "Citrus Dawn", "fragrance", "50 ml", 4900, "Fresh citrus", "Bergamot, neroli, clean musk",
     "Bergamot and neroli over a clean musk. Light, for daytime.", 0, 1, "citrus", 3, {"image": "classy-citrus.jpg"}),
    ("amber-noir", "Amber Noir", "fragrance", "50 ml", 6400, "Amber", "Amber, smoked vanilla, oud",
     "Amber and smoked vanilla with a little oud. A warm evening scent.", 2, 3, "amber", 14, {"image": "classy-amber.jpg"}),
    ("rain-oud", "Rain Oud", "fragrance", "30 ml", 5800, "Woody", "Petrichor, oud, wet stone",
     "Petrichor and oud. The short-rains limited run.", 0, 2, "rain", 9, {"image": "classy-rain.jpg", "limited": True}),
    # Invented for the sample, so search has a catalogue to rank.
    ("kahawa-crema", "Kahawa Crema", "fragrance", "50 ml", 5600, "Gourmand", "Coffee, cream, vanilla bean",
     "Roasted coffee softened with cream and vanilla bean.", 2, 2, "coffee", 6, {}),
    ("honey-baobab", "Honey Baobab", "fragrance", "30 ml", 4200, "Gourmand", "Wild honey, baobab fruit, soft vanilla",
     "Wild honey and tart baobab fruit with a little vanilla.", 1, 1, "honey", 11, {}),
    ("cocoa-velvet", "Cocoa Velvet", "fragrance", "100 ml", 8400, "Gourmand", "Dark cocoa, vanilla absolute, sandalwood",
     "Dark cocoa and rich vanilla over sandalwood.", 3, 3, "coffee", 4, {}),
    ("marula-milk-set", "Marula Milk Gift Set", "gift", "30 ml", 5900, "Gourmand", "Marula, coconut milk, vanilla",
     "A 30 ml spray of marula and coconut milk with a matching body lotion, gift-wrapped with a card.", 2, 1, "vanilla", 5, {}),
    ("lime-market", "Lime Market", "fragrance", "30 ml", 4000, "Fresh citrus", "Lime zest, ginger, vetiver",
     "Lime zest and fresh ginger with a dry vetiver base.", 0, 1, "citrus", 12, {}),
    ("tangerine-kiosk", "Tangerine Kiosk", "fragrance", "50 ml", 4600, "Fresh citrus", "Tangerine, basil, white tea",
     "Tangerine and green basil over white tea.", 0, 1, "citrus", 8, {}),
    ("neroli-terrace", "Neroli Terrace", "fragrance", "100 ml", 7600, "Fresh citrus", "Neroli, petitgrain, sea salt",
     "Orange blossom and petitgrain with a salty edge.", 0, 2, "sea", 5, {}),
    ("grapefruit-run", "Grapefruit Run", "fragrance", "50 ml", 4300, "Fresh citrus", "Pink grapefruit, mint, cedar",
     "Pink grapefruit and mint over light cedar. Good after the gym.", 0, 1, "citrus", 10, {}),
    ("citrus-travel-duo", "Citrus Travel Duo", "gift", "2 × 30 ml", 4400, "Fresh citrus", "Bergamot, neroli, lime, ginger",
     "Citrus Dawn and Lime Market in two 30 ml travel sprays, gift-wrapped with a card.", 0, 1, "citrus", 6, {}),
    ("amber-lantern", "Amber Lantern", "fragrance", "30 ml", 4800, "Amber", "Amber, cardamom, soft vanilla",
     "A softer amber with cardamom and a little vanilla.", 1, 2, "amber", 9, {}),
    ("spice-route", "Spice Route", "fragrance", "100 ml", 8900, "Amber", "Amber, clove, cinnamon, labdanum",
     "Amber and labdanum with clove and cinnamon.", 1, 3, "amber", 3, {}),
    ("myrrh-night", "Myrrh Night", "fragrance", "50 ml", 6900, "Amber", "Myrrh, benzoin, vanilla",
     "Myrrh and benzoin warmed with vanilla.", 2, 3, "smoke", 2, {}),
    ("amber-evening-set", "Amber Evening Set", "gift", "50 ml", 7200, "Amber", "Amber, smoked vanilla, oud",
     "Amber Noir 50 ml with an amber candle, gift-wrapped with a card.", 2, 3, "amber", 4, {}),
    ("highland-cedar", "Highland Cedar", "fragrance", "50 ml", 5400, "Woody", "Cedar, juniper, black pepper",
     "Cedar and juniper with black pepper.", 0, 2, "moss", 7, {}),
    ("karura-moss", "Karura Moss", "fragrance", "30 ml", 4500, "Woody", "Oakmoss, wet leaves, vetiver",
     "Oakmoss and wet leaves over vetiver.", 0, 1, "moss", 8, {}),
    ("vetiver-ember", "Vetiver Ember", "fragrance", "100 ml", 8200, "Woody", "Vetiver, smoked birch, leather",
     "Vetiver and smoked birch with soft leather.", 0, 3, "smoke", 5, {}),
    ("sandal-shore", "Sandal Shore", "fragrance", "50 ml", 6100, "Woody", "Sandalwood, sea salt, driftwood",
     "Creamy sandalwood with sea salt and driftwood.", 1, 2, "sea", 6, {}),
    ("jacaranda-walk", "Jacaranda Walk", "fragrance", "50 ml", 5500, "Floral", "Violet, jacaranda accord, iris",
     "Violet and iris, soft and powdery.", 0, 1, "violet", 7, {}),
    ("frangipani-coast", "Frangipani Coast", "fragrance", "30 ml", 4700, "Floral", "Frangipani, coconut, white musk",
     "Frangipani and coconut over white musk.", 1, 1, "rose", 9, {}),
    ("highland-rose", "Highland Rose", "fragrance", "100 ml", 7900, "Floral", "Rose, pink pepper, patchouli",
     "Rose with pink pepper and patchouli.", 0, 2, "rose", 1, {}),
    ("tuberose-night", "Tuberose Night", "fragrance", "50 ml", 6600, "Floral", "Tuberose, jasmine, vanilla",
     "Tuberose and jasmine with vanilla. Strong.", 1, 3, "violet", 0, {}),
    ("floral-discovery-set", "Floral Discovery Set", "gift", "3 × 10 ml", 5000, "Floral", "Violet, frangipani, rose",
     "Jacaranda Walk, Frangipani Coast and Highland Rose in three 10 ml vials, gift-wrapped with a card.", 0, 1, "rose", 5, {}),
]
products = []
stock = []
for i, (sku, name, sec, size, price, family, notes, desc, vanilla, strength, tone, on_hand, extra) in enumerate(P):
    attrs = {"notes": notes, "family": family, "vanilla": vanilla, "strength": strength, "gift_ready": sec == "gift", "tone": tone}
    attrs.update(extra)
    products.append({"sku": sku, "slug": sku, "section": sec, "name": name, "price": price, "description": desc,
                     "family": family, "size": size, "attrs": attrs, "sort": i})
    stock.append({"sku": sku, "on_hand": on_hand, "as_of": "m-40", "source": "stock take"})
products.append({"sku": "sample-2ml", "slug": "sample-2ml", "section": "sample", "name": "Free 2 ml sample", "price": 0,
                 "description": "A 2 ml vial of any scent, free with any order. Write the scent's name in the note for Makena at checkout.",
                 "size": "2 ml", "attrs": {"notes": "", "family": "", "vanilla": 0, "strength": 0, "gift_ready": False, "tone": "vanilla"},
                 "sort": len(P)})
write("products", products)
write("stock", stock)

write("price_lists", [
    {"version": "v31", "status": "live", "usd_rate": 129.00, "valid_from": "d-4 09:00", "valid_until": "d+6 09:00",
     "source": "Price list v31, confirmed by Makena"},
])
write("price_rules", [
    {"version": "v31", "code": "wrap", "grp": "wrap", "label": "Gift wrap with a handwritten card", "amount": 0, "meta": {}, "sort": 0},
])

write("zones", [
    {"zone": "nairobi", "label": "Nairobi", "fee": 300,
     "areas": ["Nairobi", "Westlands", "Kilimani", "Lavington", "Karen", "Kileleshwa", "Parklands", "Gigiri", "Runda", "South B", "South C", "Langata"],
     "note": "Same day when you order before 14:00. Later orders arrive the next day."},
    {"zone": "towns", "label": "Mombasa and other towns", "fee": 650,
     "areas": TOWNS,
     "note": "2–3 working days by courier."},
])

# 15 past orders across Nairobi and Mombasa; the newest are on their way.
past = [
    # customer index, zone, lines, days ago, state, rail (PayPal only, James's call)
    (1, "nairobi", [("amber-noir", 1)], 30, "delivered", "paypal"),
    (2, "towns", [("vanilla-gift-set", 2)], 28, "delivered", "paypal"),
    (3, "nairobi", [("citrus-dawn", 1), ("sample-2ml", 1)], 26, "delivered", "paypal"),
    (4, "nairobi", [("rain-oud", 1)], 24, "delivered", "paypal"),
    (5, "towns", [("highland-cedar", 1)], 22, "delivered", "paypal"),
    (6, "nairobi", [("kahawa-crema", 1), ("honey-baobab", 1)], 20, "delivered", "paypal"),
    (7, "nairobi", [("floral-discovery-set", 1)], 18, "delivered", "paypal"),
    (8, "towns", [("tangerine-kiosk", 1)], 16, "delivered", "paypal"),
    (9, "nairobi", [("amber-evening-set", 1)], 14, "delivered", "paypal"),
    (10, "towns", [("sandal-shore", 1), ("sample-2ml", 1)], 12, "delivered", "paypal"),
    (11, "nairobi", [("jacaranda-walk", 1)], 10, "delivered", "paypal"),
    (12, "towns", [("myrrh-night", 1)], 8, "delivered", "paypal"),
    (13, "nairobi", [("lime-market", 2)], 6, "delivered", "paypal"),
    (14, "towns", [("citrus-travel-duo", 1)], 2, "out", "paypal"),
    (0, "nairobi", [("vanilla-gift-set", 1)], 1, "paid", "paypal"),
]
ships_to = {ci: ("Nairobi" if zone == "nairobi" else "Mombasa") for ci, zone, *_ in past}

sample_names = [
    "Adhiambo L.", "Cheruiyot B.", "Halima S.", "Imani K.", "Jabali M.", "Kanini W.", "Lomuria E.", "Makau P.",
    "Nasimiyu J.", "Oloo D.", "Pendo A.", "Rehema N.", "Sang T.", "Tabitha G.", "Wekesa F.", "Yego R.",
    "Zuri O.", "Kagwe H.", "Msafiri C.", "Naliaka V.",
]
places = ["Nairobi", "Mombasa"]
customers = [
    {"customer_id": "cus-achieng", "name": "Achieng", "email": "achieng@example.com", "phone": "+44 7700 900512", "place": "Nairobi", "created_at": "d-12 20:05"},
]
for i, name in enumerate(sample_names, start=1):
    customers.append({"customer_id": f"cus-s{i:02d}", "name": f"{name} (Sample customer)", "email": f"classy{i:02d}@example.com",
                      "place": ships_to.get(i, places[i % 3 == 0]), "created_at": f"d-{70 - i} 10:00"})
write("customers", customers)

orders = []
for n, (ci, zone, lines, ago, state, rail) in enumerate(past, start=1):
    oid = f"CP-{2000 + n}"
    cus = customers[ci]
    name = cus["name"].replace(" (Sample customer)", "")
    if zone == "nairobi" and state == "paid":
        placed = "18:20"  # after the cut-off, so it goes out the next day (today)
    elif zone == "nairobi":
        placed = f"{9 + n % 5:02d}:20"  # 9:20 to 13:20, before the 14:00 cut-off
    else:
        placed = f"{11 + n % 7:02d}:20"
    paid = placed[:3] + "22"
    events = [
        {"step": "requested", "at": f"d-{ago} {placed}", "actor": name, "note": "Order placed"},
        {"step": "paid", "at": f"d-{ago} {paid}", "actor": "the Cashier", "note": "Paid with PayPal (sandbox) at the shop's rate."},
    ]
    if state in ("out", "delivered"):
        if zone == "nairobi":
            events.append({"step": "out", "at": f"d-{ago} 15:30", "actor": "Makena", "note": "Out for delivery."})
        else:
            events.append({"step": "out", "at": f"d-{ago - 1} 10:30", "actor": "Makena", "note": "With the courier."})
    if state == "delivered":
        when = f"d-{ago} 17:10" if zone == "nairobi" else f"d-{ago - 3} 15:10"
        events.append({"step": "delivered", "at": when, "actor": "Makena", "note": "Delivered."})
    orders.append({
        "order_id": oid, "customer": cus["customer_id"], "contact_name": name, "contact_email": cus["email"],
        "kind": "catalogue", "state": state, "fulfilment": "delivery", "zone": zone, "price_list": "v31",
        "recipient": f"{name}, {'Nairobi' if zone == 'nairobi' else 'Mombasa'}",
        "created_at": f"d-{ago} {placed}",
        "lines": [{"sku": s, "qty": q} for s, q in lines],
        "events": events,
        "payments": [{"payment_id": f"pay-c{2000 + n}", "rail": rail, "currency": "USD", "state": "sample", "at": f"d-{ago} {paid}"}],
    })
# Achieng's gift plan for her bridesmaids, requested as one order and held for Makena (the
# shop confirms big gift orders before asking for payment), and this week's orders.
orders += [
    {"order_id": "CP-2016", "customer": "cus-achieng", "contact_name": "Achieng", "contact_email": "achieng@example.com",
     "contact_phone": "+44 7700 900512", "kind": "custom", "state": "held", "fulfilment": "delivery", "zone": "nairobi",
     "price_list": "v31", "created_at": "m-120", "recipient": "Achieng, Nairobi (five bridesmaids' gifts)",
     "note": "Gift plan for five bridesmaids: wrap each with a card, deliver by Fri 4 Dec.",
     "lines": [{"sku": "citrus-dawn", "qty": 1}, {"sku": "vanilla-gift-set", "qty": 1}, {"sku": "rain-oud", "qty": 1},
               {"sku": "marula-milk-set", "qty": 1}, {"sku": "amber-lantern", "qty": 1}],
     "events": [{"step": "requested", "at": "m-120", "actor": "Achieng", "note": "Gift plan requested as one order. Held for Makena; nothing charged."}]},
    {"order_id": "CP-2017", "customer": "cus-s03", "contact_name": sample_names[2], "contact_email": "classy03@example.com",
     "kind": "catalogue", "state": "paid", "fulfilment": "delivery", "zone": "nairobi", "price_list": "v31", "created_at": "d-1 16:20",
     "recipient": f"{sample_names[2]}, Nairobi",
     "lines": [{"sku": "jacaranda-walk", "qty": 1}, {"sku": "sample-2ml", "qty": 1}],
     "events": [{"step": "requested", "at": "d-1 16:20", "actor": sample_names[2], "note": "Order placed"},
                {"step": "paid", "at": "d-1 16:22", "actor": "the Cashier", "note": "Paid with PayPal (sandbox) at the shop's rate."}],
     "payments": [{"payment_id": "pay-c2017", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-1 16:22"}]},
    {"order_id": "CP-2018", "customer": "cus-s05", "contact_name": sample_names[4], "contact_email": "classy05@example.com",
     "kind": "catalogue", "state": "out", "fulfilment": "delivery", "zone": "towns", "price_list": "v31", "created_at": "d-2 11:30",
     "recipient": f"{sample_names[4]}, Mombasa",
     "lines": [{"sku": "highland-rose", "qty": 1}],
     "events": [{"step": "requested", "at": "d-2 11:30", "actor": sample_names[4], "note": "Order placed"},
                {"step": "paid", "at": "d-2 11:32", "actor": "the Cashier", "note": "Paid with PayPal (sandbox) at the shop's rate."},
                {"step": "out", "at": "d-1 10:00", "actor": "Makena", "note": "With the courier."}],
     "payments": [{"payment_id": "pay-c2018", "rail": "paypal", "currency": "USD", "state": "sample", "at": "d-2 11:32"}]},
]
write("orders", orders)

write("refunds", [
    {"refund_id": "R-2008-1", "order_id": "CP-2008", "amount": 2300, "reason": "The Tangerine Kiosk bottle leaked on the way to Mombasa. The customer asked for half back.",
     "state": "waiting", "at": "m-180"},
])
write("questions", [
    {"question_id": "q-c1", "desk": "Fragrances", "question": "Is Amber Noir vegan?", "page": "/search", "at": "d-1 16:12"},
    {"question_id": "q-c2", "desk": "Fragrances", "question": "How long does Rain Oud last on skin?", "contact": "classy05@example.com", "page": "/fragrances/rain-oud", "at": "m-260"},
    {"question_id": "q-c3", "desk": "Gifts", "question": "Can you engrave a name on the Amber Noir bottle?", "contact": "classy09@example.com", "page": "/gifts", "at": "m-70"},
])
print("wrote", os.path.normpath(OUT))
