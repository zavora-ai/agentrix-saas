// Kilele Bakery, "The same Saturday, twice": the home page's film (split.html?f=kilele).
// Left: W11's stills (film_01..16 in agentrix-media), fictional people made with AI.
// Right: the real product (right/*.webp, from capture.py) and panels drawn below for the old
// WordPress site, Telegram, reviews, a chat, the prep list (coming next) and two stores.
window.FILMDEF = {
  id: "kilele",
  title: "The same Saturday, twice",
  sample: "Kilele Bakery is a sample business.",
  where: "Kilele Bakery, Westlands",
  // Where each still happens, when it isn't the Westlands shop.
  places: { l03: "Amani's home", l07: "Another bakery, across town", l11: "Amani's home", l15: "Amani's home", l16: "Kilele Bakery, Kilimani" },
  // Stills drawn brighter than their hour: pulled down so night reads as night.
  night: ["l08"],
  rlabel: { before: "Kilele's website today", after: "Kilele's website on Agentrix" },
  // W11's stills live in agentrix-media; stills/ holds a local copy until W11's branch reaches platform.
  stills: k => [`../../agentrix-media/samples/agentrix-home/film_${k.slice(1)}.webp`, `stills/${k}.webp`],
  shots: {
  l01: "8 a.m. A queue under Kilele's coral awning.", l02: "The rush. The laptop under the counter stays shut.", l03: "Amani at home, frowning at her phone.", l04: "6 p.m. Wanjiru pulls the shutter down.", l05: "Night. The shop is dark.", l06: "Sunday, 7 a.m. Tickets everywhere.", l07: "Amani walks into another bakery.", l08: "Late night. Wanjiru and her laptop.", l09: "8 a.m. again. Calm.", l10: "Wanjiru at the oven, answering on her phone.", l11: "Amani and Zawadi design the cake.", l12: "6 p.m. Shutter down. Smiles.", l13: "Sunday, 7 a.m. Everything boxed and labelled.", l14: "Amani collects, on time.", l15: "Zawadi's party.", l16: "Months later: a second Kilele.",
},
  images: Object.fromEntries(["site", "studio-2", "studio-4", "console", "decisions", "assess-start", "assess", "move", "move-left"].map(k => [k, `right/${k}.webp`])),
  // The old WordPress dashboard's lists, revealed as the counts climb.
  wp: {
    orders: [["#1041", "Grace M.", "2 sourdough", "09:12"], ["#1042", "Otieno P.", "Croissants ×6", "10:40"], ["#1043", "Joyce K.", "Birthday cake", "11:02"], ["#1044", "Wambui N.", "Banana bread", "11:30"], ["#1045", "Kevin O.", "Cinnamon rolls", "13:05"], ["#1046", "Amani W.", "Custom cake?", "14:14"], ["#1047", "Faith A.", "Red velvet", "17:20"], ["#1048", "Brian K.", "Sourdough ×2", "18:45"], ["#1049", "Lucy W.", "Weekend box", "19:30"], ["#1050", "Sam M.", "Fruit cake", "21:10"], ["#1051", "Mercy N.", "Croissants ×12", "22:05"], ["#1052", "Ian T.", "Rye loaf", "22:40"], ["#1053", "Achieng O.", "Cake for 30", "23:15"], ["#1054", "Peter K.", "Mandazi ×20", "23:38"]],
    msgs: [["Do you deliver to Kilimani?", "Wambui N. · 10:52"], ["Wedding cake for 120?", "Mary K. · 11:20"], ["Personalised birthday cake for Zawadi: gluten-free, her name in gold. For Saturday?", "Amani W. · 14:14"], ["Are you open Sunday?", "Kevin O. · 17:02"], ["Price for 3 tiers?", "Faith A. · 18:30"], ["Can I collect at 7 a.m.?", "Brian K. · 19:10"], ["Nut-free options?", "Lucy W. · 20:40"], ["Corporate order, 60 boxes", "Sam M. · 21:55"], ["Is my order confirmed?", "Achieng O. · 23:30"]],
  },
  templates: {
    "wp": `<div class="wp">
  <div class="bar"><span class="home">Kilele Bakery</span><span>+ New</span><span class="spacer"></span><span>Howdy, admin</span></div>
  <nav class="side">
    <div class="on">Dashboard</div><div>Posts</div><div>Pages</div>
    <div>Orders <em data-n="orders">0</em></div>
    <div>Contact forms <em data-n="msgs">0</em></div>
    <div>Plugins <em>3</em></div><div>Settings</div>
  </nav>
  <main>
    <h2>Dashboard</h2>
    <div class="notice">3 plugins have updates waiting.</div>
    <div class="notice red" data-alarm>Unread: <span data-n="orders">0</span> orders and <span data-n="msgs">0</span> messages.</div>
    <div class="grid">
      <div class="box"><h3>Visitors today</h3><div class="big" data-n="visits">0</div><div class="lbl">people on the site</div></div>
      <div class="box"><h3>New orders</h3><div class="big" data-n="orders">0</div><div class="lbl">not opened</div></div>
      <div class="box full"><h3>Recent orders</h3><table data-list="orders"></table></div>
      <div class="box full msgs" data-list="msgs"><h3>Contact form</h3></div>
    </div>
  </main>
</div>`,
    "telegram": `<div class="tg">
  <div class="head"><img src="marks/chief.svg" alt=""><div><b>Kilele Bakery · WebAgents</b><span>bot</span></div></div>
  <div class="msgs">
    <div class="m in" data-at="0">Amani W. is designing a cake on the site: 2 tiers, vanilla sponge, for Saturday.<br><br>She asks: can you write <b>“Zawadi”</b> in gold on top? The site doesn't say.<div class="btns"><span>Reply</span><span>Pass</span></div><small>14:16</small></div>
    <div class="m out" data-at="1.8">Yes, gold lettering is fine.<small>14:17 ✓✓</small></div>
    <div class="m in" data-at="3.2">Sent to Amani. Her order is waiting for your OK.<div class="btns"><span>Open in console</span></div><small>14:17</small></div>
  </div>
</div>`,
    "review1": `<div class="panel"><h3>Reviews</h3>
  <div class="card"><div class="who"><i style="background:#8e8da3">J</i>Joyce K.</div><div class="stars low">★<span class="off">★★★★</span></div>
  <p class="quote">Ordered a birthday cake on their website on Saturday night. Nobody confirmed. Had to buy one somewhere else.</p></div>
</div>`,
    "review5": `<div class="panel"><h3>Reviews</h3>
  <div class="card"><div class="who"><i style="background:#de4060">A</i>Amani W.</div><div class="stars">★★★★★</div>
  <p class="quote">Designed Zawadi's cake on their site, got an answer about the gold lettering in minutes, and it was ready on time.</p></div>
</div>`,
    "share": `<div class="chat">
  <div class="head">Westlands parents<span>38 members</span></div>
  <div class="msgs">
    <div class="m"><b>Amani</b>Kilele Bakery did Zawadi's cake. You can design yours on their site, even at night 🎂
      <div class="link"><img src="right/site.webp" alt=""><div>Kilele Bakery<span>Design a cake · Westlands</span></div></div></div>
    <div class="m"><b>Wambui</b>Ordering for Kip's party now!</div>
  </div>
</div>`,
    "prep": `<div class="panel prep">
  <h3>Tomorrow's prep <span class="chipx" style="background:#fff4dd;color:#8a5300;vertical-align:middle">Coming next</span></h3>
  <p style="font-size:22px;color:#5f5e73;margin-top:8px">Sunday · from 14 orders · ready at 5:30 a.m.</p>
  <div class="card" style="padding:22px 30px">
    <h4 style="margin-top:0">Flour and sugar</h4><table><tr><td>Bread flour</td><td>18 kg</td></tr><tr><td>Cake flour</td><td>6 kg</td></tr></table>
    <h4>Dairy and eggs</h4><table><tr><td>Butter</td><td>6 kg</td></tr><tr><td>Eggs</td><td>12 trays</td></tr></table>
    <h4>Packaging</h4><table><tr><td>Cake boxes</td><td>14</td></tr></table>
    <h4>Collections</h4><table><tr><td>7:30 · Grace M. · 2 sourdough</td><td>✓</td></tr><tr><td>10:00 · Amani W. · 2-tier vanilla, gold lettering</td><td>✓</td></tr></table>
  </div>
</div>`,
    "sites": `<div class="panel sites"><h3>Your sites</h3>
  <div class="card s"><img src="marks/chief.svg" alt=""><div><b>Kilele Bakery</b><span>Westlands · live</span></div><span class="chipx" style="background:#e6f5ee;color:#11734a">Live</span></div>
  <div class="card s"><img src="marks/chief.svg" alt=""><div><b>Kilele Bakery</b><span>Kilimani · opened this month</span></div><span class="chipx" style="background:#e6f5ee;color:#11734a">Live</span></div>
</div>`,
  },
  film: [
  { at: 0, len: 8, mood: "before", clock: "Saturday · 8:00 a.m.", left: "l01", right: { kind: "wp", visits: [37, 126], orders: [0, 0], msgs: [0, 0] }, cap: "Saturday at Kilele Bakery. The shop is full. So is the website." },
  { at: 8, len: 8, mood: "before", clock: "Saturday · 11:02 a.m.", left: "l02", right: { kind: "wp", visits: [126, 214], orders: [0, 4], msgs: [0, 2] }, cap: "Orders come in. Nobody sees them." },
  { at: 16, len: 9, mood: "before", clock: "Saturday · 2:14 p.m.", left: "l03", right: { kind: "wp", visits: [214, 288], orders: [4, 6], msgs: [3, 3], flash: "Zawadi" }, cap: "Amani asks about a cake for her daughter. Her question lands in an inbox nobody opens." },
  { at: 25, len: 8, mood: "before", clock: "Saturday · 6:00 p.m.", left: "l04", right: { kind: "wp", visits: [288, 340], orders: [6, 9], msgs: [3, 6] }, cap: "The shop closes. The website doesn't know." },
  { at: 33, len: 6, mood: "before", clock: "Saturday · 11:40 p.m.", left: "l05", right: { kind: "wp", visits: [340, 402], orders: [9, 14], msgs: [6, 9] }, cap: "People keep visiting, ordering and asking." },
  { at: 39, len: 8, mood: "before", clock: "Sunday · 7:00 a.m.", left: "l06", right: { kind: "wp", visits: [402, 402], orders: [14, 14], msgs: [9, 9], alarm: true }, cap: "Sunday morning: fourteen orders nobody planned for." },
  { at: 47, len: 7, mood: "before", clock: "Sunday · 9:30 a.m.", left: "l07", right: { kind: "tpl", key: "review1" }, cap: "Some customers waited. Some went elsewhere." },
  { at: 54, len: 12, mood: "move", clock: "Sunday · 10:48 p.m.", left: "l08", right: { kind: "seq", items: [{ key: "assess-start", from: 0, to: 1.8 }, { key: "assess", from: 1.8, to: 5 }, { key: "move", from: 5, to: 8.5 }, { key: "move-left", from: 8.5, to: 12 }] }, cap: [[0, 5, "That night, Wanjiru found Agentrix. A free check-up first."], [5, 12, "Then the Move. The old site's problems stay behind."]] },
  { at: 66, len: 6, mood: "after", clock: "Saturday · 8:00 a.m.", left: "l09", right: { kind: "img", key: "site", zoom: [1, 1.06], origin: "50% 20%" }, title: "Same Saturday.", cap: "" },
  { at: 72, len: 8, mood: "after", clock: "Saturday · 11:02 a.m.", left: "l09", right: { kind: "img", key: "console", zoom: [1, 1.3], origin: "92% 30%" }, cap: "Orders land in one list, each with a time to collect." },
  { at: 80, len: 12, mood: "after", clock: "Saturday · 2:14 p.m.", left: [{ key: "l11", from: 0, to: 7 }, { key: "l10", from: 7, to: 12 }], right: { kind: "seq", items: [{ key: "studio-2", from: 0, to: 3.5 }, { key: "studio-4", from: 3.5, to: 7 }, { key: "telegram", from: 7, to: 12 }] }, cap: [[0, 7, "Amani designs Zawadi's cake from what Kilele actually makes."], [7, 12, "What the site doesn't know, it asks Wanjiru on Telegram. She answers between batches."]] },
  { at: 92, len: 7, mood: "after", clock: "Saturday · 6:00 p.m.", left: "l12", right: { kind: "img", key: "decisions", zoom: [1, 1.12], origin: "40% 30%" }, cap: "The shop closes. The website keeps working." },
  { at: 99, len: 8, mood: "after", clock: "Saturday · 11:40 p.m.", left: "l05", right: { kind: "tpl", key: "prep" }, cap: "By morning, tomorrow's work is planned." },
  { at: 107, len: 8, mood: "after", clock: "Sunday · 7:00 a.m.", left: "l13", right: { kind: "img", key: "console", zoom: [1, 1.18], origin: "30% 20%" }, cap: "Sunday morning. Nobody's surprised." },
  { at: 115, len: 7, mood: "after", clock: "Sunday · 10:00 a.m.", left: "l14", right: { kind: "tpl", key: "review5" }, cap: "Amani's cake, on time." },
  { at: 122, len: 6, mood: "after", clock: "Sunday · 4:00 p.m.", left: "l15", right: { kind: "tpl", key: "share" }, cap: "And she tells her friends." },
  { at: 128, len: 8, mood: "after", clock: "Months later", left: "l16", right: { kind: "tpl", key: "sites" }, cap: "Kilele Bakery, now in Westlands and Kilimani." },
  { at: 136, len: 8, mood: "end", end: true },
],
  // The home page hero: 19 s, silent, words burnt in, loops back to the start.
  hero: [
    { at: 0, len: 5, mood: "before", clock: "Saturday · 6:00 p.m.", left: "l04", right: { kind: "wp", visits: [288, 340], orders: [6, 9], msgs: [3, 6] }, cap: "The shop closes at six. The orders don't stop." },
    { at: 5, len: 4.5, mood: "before", clock: "Saturday · 11:40 p.m.", left: "l05", right: { kind: "wp", visits: [340, 402], orders: [9, 14], msgs: [6, 9], alarm: true }, cap: "By midnight: fourteen orders, nine questions, nobody answering." },
    { at: 9.5, len: 5, mood: "after", clock: "Saturday · 6:00 p.m.", left: "l12", right: { kind: "img", key: "decisions", zoom: [1, 1.12], origin: "40% 30%" }, cap: "Agentrix WebAgents are proactive, responsive and supportive of customer needs." },
    { at: 14.5, len: 4.5, mood: "after", clock: "Saturday · 6:10 p.m.", left: "l10", right: { kind: "tpl", key: "telegram" }, cap: "What they can't answer, they ask Wanjiru." },
  ],
  loop: [
  { at: 0, len: 6, mood: "before", clock: "Saturday · 6:00 p.m.", left: "l04", right: { kind: "wp", visits: [288, 402], orders: [6, 14], msgs: [3, 9] }, cap: "The shop closes at six. The orders don't stop." },
  { at: 6, len: 6, mood: "after", clock: "Saturday · 6:00 p.m.", left: "l12", right: { kind: "img", key: "decisions", zoom: [1, 1.12], origin: "40% 30%" }, cap: "Now your WebAgents take it from there." },
  { at: 12, len: 4, mood: "end", end: true },
],
};
