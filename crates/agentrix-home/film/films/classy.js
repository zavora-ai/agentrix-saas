// Classy Perfumes, "The gift at eleven at night" (split.html?f=classy).
// Use cases: helping people choose (search that answers, honest fit), a gift plan for a group,
// shoppers' own AI assistants buying through the agent door, and the owner confirming.
// Left: W11's stills (classy_01..12), fictional people made with AI. Right: the real sample
// site and onboarding pages (right/classy/), and panels drawn here for the old shop, the
// assistant on Brian's phone, Telegram and a review.
const AI_BEFORE = `
<div class="phonebg"><div class="phone"><div class="screen"><div class="ai">
  <div class="head">Assistant<span>on Brian's phone</span></div>
  <div class="msgs">
    <div class="u" data-at="0.3">A vanilla perfume gift under KSh 5,000 in Westlands, delivered tomorrow?</div>
    <div class="a" data-at="2">Classy Perfumes's site doesn't show stock or prices I can read, so I can't check it.<br><br>Another shop lists a vanilla gift set at KSh 4,800 with delivery tomorrow. Order there?</div>
    <div class="u" data-at="5">Yes, go ahead.</div>
  </div>
</div></div></div></div>`;

const AI_AFTER = `
<div class="phonebg"><div class="phone"><div class="screen"><div class="ai">
  <div class="head">Assistant<span>on Brian's phone</span></div>
  <div class="msgs">
    <div class="u" data-at="0.3">A vanilla perfume gift under KSh 5,000 in Westlands, delivered tomorrow?</div>
    <div class="a" data-at="1.8">I asked Classy Perfumes through its agent door.<br><br><b>Vanilla Gift Set, 30 ml: KSh 4,500.</b> 7 in stock, gift-wrapped, delivery tomorrow in Westlands.
      <div class="src"><b>Classy Perfumes · your bag</b><span>Review and pay on the shop's site</span></div>
      <small>Prices and stock from the shop's own records</small></div>
  </div>
</div></div></div></div>`;

window.FILMDEF = {
  id: "classy",
  title: "The gift at eleven at night",
  sample: "Classy Perfumes is a sample business.",
  where: "Classy Perfumes, Westlands",
  places: { c02: "Achieng's home", c03: "A café in Westlands", c04: "Brian's home", c08: "Achieng's home", c09: "Brian's home", c11: "A garden wedding, Karen" },
  night: ["c06"],
  rlabel: { before: "Classy's website today", after: "Classy's website on Agentrix" },
  stills: k => [`../../agentrix-media/samples/agentrix-home-classy/classy_${k.slice(1)}.webp`, `stills/classy/${k}.webp`],
  shots: {
    c01: "Friday evening. Makena wraps gifts at the counter.", c02: "Achieng plans her bridesmaids' gifts.", c03: "Achieng sends a message.",
    c04: "11 p.m. Brian asks his phone.", c05: "Saturday morning. Makena reads the messages, too late.", c06: "Late night. Makena and her laptop.",
    c07: "Same Friday. Calm at the counter.", c08: "Achieng, relieved.", c09: "Brian, sorted.", c10: "Makena confirms the plan.",
    c11: "The wedding: five bridesmaids, five gifts.", c12: "Months later: a full shop.",
  },
  images: Object.fromEntries(["site", "search", "plan", "agents", "assess-start", "assess", "move"].map(k => [k, `right/classy/${k}.webp`])),
  wp: {
    orders: [["#2101", "Lucy W.", "Body lotion", "09:40"], ["#2102", "Kevin O.", "Oud 50 ml", "12:15"], ["#2103", "Mercy N.", "Gift wrapping", "15:30"], ["#2104", "Ian T.", "Travel spray", "18:05"]],
    msgs: [["Do you have this in 100 ml?", "Faith A. · 16:20"], ["Five gift sets for my bridesmaids, wrapped, for Sat 12 Dec? Budget KSh 25,000.", "Achieng O. · 21:40"], ["Is the oud in stock?", "Sam M. · 22:30"], ["Never mind, I found them elsewhere.", "Achieng O. · 07:55"]],
  },
  templates: {
    wp: `
<div class="wp">
  <div class="bar"><span class="home">Classy Perfumes</span><span>+ New</span><span class="spacer"></span><span>Howdy, admin</span></div>
  <nav class="side">
    <div class="on">Dashboard</div><div>Posts</div><div>Products</div>
    <div>Orders <em data-n="orders">0</em></div>
    <div>Contact forms <em data-n="msgs">0</em></div>
    <div>Plugins <em>5</em></div><div>Settings</div>
  </nav>
  <main>
    <h2>Dashboard</h2>
    <div class="notice">5 plugins have updates waiting.</div>
    <div class="notice red" data-alarm>Unread: <span data-n="orders">0</span> orders and <span data-n="msgs">0</span> messages.</div>
    <div class="grid">
      <div class="box"><h3>Visitors today</h3><div class="big" data-n="visits">0</div><div class="lbl">people on the site</div></div>
      <div class="box"><h3>New orders</h3><div class="big" data-n="orders">0</div><div class="lbl">not opened</div></div>
      <div class="box full"><h3>Recent orders</h3><table data-list="orders"></table></div>
      <div class="box full msgs" data-list="msgs"><h3>Contact form</h3></div>
    </div>
  </main>
</div>`,
    oldsearch: `
<div class="oldsite">
  <div class="mast">Classy Perfumes<small>Fragrance &amp; gifts · Westlands</small></div>
  <div class="nav"><span>Home</span><span>Shop</span><span>About</span><span>Contact</span></div>
  <div class="main">
    <div class="search">gifts for five bridesmaids under 25000</div>
    <h1 style="font-size:34px">Search results for “gifts for five bridesmaids under 25000”</h1>
    <p class="meta">Showing 0 results</p>
    <div class="empty">No products were found matching your selection.</div>
  </div>
</div>`,
    "ai-before": AI_BEFORE,
    "ai-after": AI_AFTER,
    telegram: `
<div class="tg">
  <div class="head"><img src="marks/chief.svg" alt=""><div><b>Classy Perfumes · WebAgents</b><span>bot</span></div></div>
  <div class="msgs">
    <div class="m in" data-at="0.2">Achieng O. asks for her bridesmaids' plan: <b>5 gift sets, wrapped</b>, KSh 23,900, for Sat 12 Dec.<br><br>Every set is in stock. Nothing is charged until you confirm.<div class="btns"><span>Open to confirm</span><span>See the plan</span></div><small>08:02</small></div>
    <div class="m in" data-at="3">Confirmed with your passkey. Achieng has her link to pay.<small>08:04</small></div>
  </div>
</div>`,
    review5: `
<div class="panel"><h3>Reviews</h3>
  <div class="card"><div class="who"><i style="background:#6b2d5c">A</i>Achieng O.</div><div class="stars">★★★★★</div>
  <p class="quote">Planned five bridesmaids' gifts at 9 p.m. on a Friday. All wrapped, all delivered the day before the wedding.</p></div>
</div>`,
    share: `
<div class="chat">
  <div class="head">Kilimani book club<span>24 members</span></div>
  <div class="msgs">
    <div class="m" data-at="0.3"><b>Achieng</b>Classy Perfumes did all my bridesmaids' gifts. Their site actually helps you choose, even at night.
      <div class="link"><img src="right/classy/site.webp" alt=""><div>Classy Perfumes<span>Perfume and gift sets · Westlands</span></div></div></div>
    <div class="m" data-at="2.4"><b>Kawira</b>My assistant just ordered my sister's birthday gift there 😄</div>
  </div>
</div>`,
  },
  film: [
    { at: 0, len: 7, mood: "before", clock: "Friday · 5:30 p.m.", left: "c01", right: { kind: "wp", visits: [64, 180], orders: [0, 3], msgs: [0, 1] }, cap: "Friday at Classy Perfumes. Makena wraps every gift by hand." },
    { at: 7, len: 8, mood: "before", clock: "Friday · 9:10 p.m.", left: "c02", right: { kind: "tpl", key: "oldsearch" }, cap: "Achieng needs gifts for five bridesmaids. The shop's search finds nothing." },
    { at: 15, len: 7, mood: "before", clock: "Friday · 9:40 p.m.", left: "c03", right: { kind: "wp", visits: [180, 236], orders: [3, 3], msgs: [1, 2], flash: "bridesmaids" }, cap: "So she asks by message. It waits in an inbox." },
    { at: 22, len: 9, mood: "before", clock: "Friday · 11:15 p.m.", left: "c04", right: { kind: "tpl", key: "ai-before" }, cap: "Brian asks his AI assistant. It can't read the shop's site, so it sends him elsewhere." },
    { at: 31, len: 7, mood: "before", clock: "Saturday · 8:00 a.m.", left: "c05", right: { kind: "wp", visits: [236, 236], orders: [3, 4], msgs: [3, 4], alarm: true }, cap: "By morning, both gifts were bought somewhere else." },
    { at: 38, len: 12, mood: "move", clock: "Saturday · 10:30 p.m.", left: "c06", right: { kind: "seq", items: [{ key: "assess-start", from: 0, to: 2 }, { key: "assess", from: 2, to: 5.5 }, { key: "move", from: 5.5, to: 12 }] }, cap: [[0, 5.5, "That night, Makena found Agentrix. A free check-up first."], [5.5, 12, "Then the Move: the same shop, with a site that helps people choose."]] },
    { at: 50, len: 6, mood: "after", clock: "Friday · 5:30 p.m.", left: "c07", right: { kind: "img", key: "site", zoom: [1, 1.06], origin: "50% 20%" }, title: "Same Friday.", cap: "" },
    { at: 56, len: 9, mood: "after", clock: "Friday · 9:10 p.m.", left: "c08", right: { kind: "seq", items: [{ key: "search", from: 0, to: 4.5 }, { key: "plan", from: 4.5, to: 9 }] }, cap: [[0, 4.5, "The site answers the question she actually asked, and says what's over budget."], [4.5, 9, "For the bridesmaids, it drafts a plan from what Makena has in stock."]] },
    { at: 65, len: 9, mood: "after", clock: "Friday · 11:15 p.m.", left: "c09", right: { kind: "seq", items: [{ key: "ai-after", from: 0, to: 5 }, { key: "agents", from: 5, to: 9 }] }, cap: [[0, 5, "Brian's AI assistant asks the shop itself, through its agent door."], [5, 9, "Same prices, same stock, same rules. A person always pays."]] },
    { at: 74, len: 8, mood: "after", clock: "Saturday · 8:00 a.m.", left: "c10", right: { kind: "tpl", key: "telegram" }, cap: "Makena checks the plan and confirms it. Nothing is charged until she does." },
    { at: 82, len: 8, mood: "after", clock: "Saturday 12 Dec · 2:00 p.m.", left: "c11", right: { kind: "tpl", key: "review5" }, cap: "Five bridesmaids, five gifts, on time." },
    { at: 90, len: 8, mood: "after", clock: "Months later", left: "c12", right: { kind: "tpl", key: "share" }, cap: "Classy Perfumes, open at eleven at night: for people, and for their AI assistants." },
    { at: 98, len: 8, mood: "end", end: true },
  ],
};
