// Nyota Salon, "The site that sent clients to a casino" (split.html?f=salon).
// Use case: a hacked WordPress. The check-up finds what a fixer missed, the Move reads the old
// site in sealed rooms and leaves the infection behind, going live waits for the owner and can
// be undone for 30 days; then the WebAgents answer clients and pass bookings to the owner.
// Nyota has no sample site: its old site is Zavora's infected fixture, so the right side shows
// the real check-up and Move (right/salon/) and marks every drawn view of its new site as an
// illustration. Left: W11's stills (salon_01..11), fictional people made with AI.
const ILL = `<span class="chip-ill">Illustration</span>`;
const serp = (warn) => `
<div class="serp">
  <div class="q">nyota salon kilimani</div>
  <div class="r"><div class="u">nyota-salon.example › bookings</div><h4>Nyota Salon · Hair &amp; Beauty · Kilimani</h4>
    ${warn ? `<div class="warn">This site may be hacked.</div>` : ""}
    <p>Braids, cuts, colour and natural hair care in Kilimani. Open Monday to Saturday${warn ? "…" : ", 9 a.m. to 7 p.m. Book online at any hour."}</p></div>
  <div class="r dim"><div class="u">maps</div><h4>Hair salons near Kilimani</h4><p>12 places · open now</p></div>
</div>`;

window.FILMDEF = {
  id: "salon",
  title: "The site that sent clients to a casino",
  sample: "Nyota Salon is a fictional business.",
  where: "Nyota Salon, Kilimani",
  places: { s06: "Auma's kitchen table" },
  night: ["s06"],
  rlabel: { before: "Nyota's website today", after: "Nyota's website on Agentrix" },
  stills: k => [`../../agentrix-media/samples/agentrix-home-salon/salon_${k.slice(1)}.webp`, `stills/salon/${k}.webp`],
  shots: {
    s01: "Monday 9 a.m. The chairs should be full.", s02: "“Your website sent me to a casino.”", s03: "Auma sees the warning.",
    s04: "A fixer restores a backup.", s05: "Two weeks later: closing early.", s06: "Late night. Auma and her laptop.",
    s07: "Next Monday: every chair full.", s08: "Auma answers between clients.", s09: "2 a.m. The salon is closed.",
    s10: "Grace's new braids.", s11: "Months later: two new stylists.",
  },
  images: Object.fromEntries(["assess-start", "assess", "move", "move-left", "move-rooms"].map(k => [k, `right/salon/${k}.webp`])),
  templates: {
    "wp-quiet": `
<div class="wp">
  <div class="bar"><span class="home">Nyota Salon</span><span>+ New</span><span class="spacer"></span><span>Howdy, admin</span></div>
  <nav class="side"><div class="on">Dashboard</div><div>Posts</div><div>Bookings <em>3</em></div><div>Contact forms</div><div>Plugins <em>7</em></div><div>Settings</div></nav>
  <main>
    <h2>Dashboard</h2>
    <div class="notice red">1 plugin has a known security problem. Update now.</div>
    <div class="grid">
      <div class="box"><h3>Bookings this week</h3><div class="big" data-from="11" data-to="3">3</div><div class="lbl">usually about 40</div></div>
      <div class="box"><h3>Visitors today</h3><div class="big" data-from="58" data-to="21">21</div><div class="lbl">people on the site</div></div>
    </div>
  </main>
</div>`,
    spam: `
<div class="phonebg"><div class="phone"><div class="screen"><div class="spam">
  <div class="url">nyota-salon.example → <b>spin-and-win.test</b></div>
  <div class="body"><h5>YOU WON!<br>SPIN NOW</h5><p>Congratulations, lucky visitor. Claim your prize in the next 60 seconds.</p><span class="btn">CLAIM</span></div>
</div></div></div></div>`,
    "serp-warn": serp(true),
    "serp-clean": serp(false),
    restored: `
<div class="wp">
  <div class="bar"><span class="home">Nyota Salon</span><span class="spacer"></span><span>Howdy, admin</span></div>
  <nav class="side"><div>Dashboard</div><div>Posts</div><div>Bookings</div><div>Plugins</div><div class="on">Backups</div><div>Settings</div></nav>
  <main>
    <h2>Backups</h2>
    <div class="notice" style="border-left-color:#00a32a">Site restored from the backup of 2 October. ✓</div>
    <div class="box"><table><tr><td>2 October, 03:00</td><td>Full site</td><td class="status">Restored</td></tr><tr><td>25 September, 03:00</td><td>Full site</td><td></td></tr><tr><td>18 September, 03:00</td><td>Full site</td><td></td></tr></table></div>
  </main>
</div>`,
    alert: `
<div class="wp">
  <div class="bar"><span class="home">Nyota Salon</span><span class="spacer"></span><span>Howdy, admin</span></div>
  <nav class="side"><div>Dashboard</div><div>Posts</div><div>Bookings</div><div class="on">Security</div><div>Plugins</div><div>Settings</div></nav>
  <main>
    <h2>Security scan</h2>
    <div class="notice red" data-at="0.4">47 hidden links found in your posts.</div>
    <div class="notice red" data-at="1.4">Unknown file in uploads: 2026/10/cache.php</div>
    <div class="notice red" data-at="2.4">Visitors from search are being redirected.</div>
  </main>
</div>`,
    ask: `
<div class="panel" style="position:relative">${ILL}<h3>Ask Nyota Salon</h3>
  <div class="card">
    <p class="quote" style="text-align:right"><span class="chipx" style="background:#1f1d3d;color:#fff;font-size:24px;padding:12px 18px">Do you do knotless braids on Sunday?</span></p>
    <p class="quote" data-at="1">Yes, knotless braids take about five hours, from the salon's price list. Sundays are by request, so I'll ask Auma and tell you here.</p>
    <p data-at="2" style="margin-top:18px;font-size:20px;color:#5f5e73">From: Prices and hours · Braids</p>
    <p data-at="2.6" style="margin-top:18px"><span class="chipx" style="background:#b72d50;color:#fff;font-size:22px;padding:10px 18px">Pass it to Auma</span></p>
  </div>
</div>`,
    telegram: `
<div class="tg">
  <div class="head"><img src="marks/chief.svg" alt=""><div><b>Nyota Salon · WebAgents</b><span>bot</span></div></div>
  <div class="msgs">
    <div class="m in" data-at="0.2">Grace W. asks for knotless braids on <b>Sunday at 10 a.m.</b> Sundays need your yes.<div class="btns"><span>Reply</span><span>Pass</span></div><small>09:05</small></div>
    <div class="m out" data-at="1.8">Yes, 10 a.m. is fine. Kendi will do it.<small>09:06 ✓✓</small></div>
    <div class="m in" data-at="3.2">Sent to Grace. Her booking is waiting for your OK.<div class="btns"><span>Open in console</span></div><small>09:06</small></div>
  </div>
</div>`,
    night: `
<div class="panel" style="position:relative">${ILL}<h3>Last night</h3>
  <div class="card">
    <p class="quote"><b data-from="0" data-to="214">214</b> requests for <code>/wp-login.php</code> and <code>/xmlrpc.php</code>.</p>
    <p class="quote" data-at="1.6">There's no WordPress here any more, so they reached nothing.</p>
  </div>
</div>`,
    review5: `
<div class="panel"><h3>Reviews</h3>
  <div class="card"><div class="who"><i style="background:#0f766e">G</i>Grace W.</div><div class="stars">★★★★★</div>
  <p class="quote">Asked at midnight, booked by morning. The braids are perfect.</p></div>
</div>`,
  },
  film: [
    { at: 0, len: 7, mood: "before", clock: "Monday · 9:00 a.m.", left: "s01", right: { kind: "tpl", key: "wp-quiet" }, cap: "Monday at Nyota Salon. The chairs should be full." },
    { at: 7, len: 8, mood: "before", clock: "Monday · 9:20 a.m.", left: "s02", right: { kind: "tpl", key: "spam" }, cap: "“Your website sent me to a casino.”" },
    { at: 15, len: 8, mood: "before", clock: "Monday · 12:00 p.m.", left: "s03", right: { kind: "tpl", key: "serp-warn" }, cap: "Search results warn people away. Bookings dry up." },
    { at: 23, len: 8, mood: "before", clock: "A week later", left: "s04", right: { kind: "tpl", key: "restored" }, cap: "A fixer restores a backup. The backup was infected too." },
    { at: 31, len: 7, mood: "before", clock: "Two weeks later", left: "s05", right: { kind: "tpl", key: "alert" }, cap: "Two weeks later, the spam is back." },
    { at: 38, len: 14, mood: "move", clock: "Two weeks later · 10:00 p.m.", left: "s06", right: { kind: "seq", items: [{ key: "assess-start", from: 0, to: 2 }, { key: "assess", from: 2, to: 5.5 }, { key: "move-rooms", from: 5.5, to: 9.5 }, { key: "move-left", from: 9.5, to: 14 }] }, cap: [[0, 5.5, "Auma pastes her address. The check-up finds what the fixer missed."], [5.5, 9.5, "The Move reads the old site in sealed rooms: no network, no keys."], [9.5, 14, "The infection stays behind. The new site goes live when she approves it."]] },
    { at: 52, len: 7, mood: "after", clock: "10:40 p.m.", left: "s06", right: { kind: "img", key: "move", zoom: [1, 1.06], origin: "50% 10%" }, cap: "And for 30 days, going back is one step." },
    { at: 59, len: 7, mood: "after", clock: "Next Monday · 9:00 a.m.", left: "s07", right: { kind: "tpl", key: "ask", label: "Nyota's website on Agentrix · illustration" }, title: "Next Monday.", cap: "" },
    { at: 66, len: 8, mood: "after", clock: "Monday · 9:05 a.m.", left: "s08", right: { kind: "tpl", key: "telegram" }, cap: "Clients ask the site at any hour. What needs Auma reaches Auma, between clients." },
    { at: 74, len: 7, mood: "after", clock: "Tuesday · 2:00 a.m.", left: "s09", right: { kind: "tpl", key: "night", label: "Nyota's website on Agentrix · illustration" }, cap: "Bots still knock on the old WordPress doors. There's nothing behind them now." },
    { at: 81, len: 7, mood: "after", clock: "Saturday · 3:00 p.m.", left: "s10", right: { kind: "tpl", key: "review5" }, cap: "Clients book again, at any hour." },
    { at: 88, len: 8, mood: "after", clock: "Months later", left: "s11", right: { kind: "tpl", key: "serp-clean" }, cap: "Nyota Salon: clean, busy, and booked a week ahead." },
    { at: 96, len: 8, mood: "end", end: true },
  ],
};
