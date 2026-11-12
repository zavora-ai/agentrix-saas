// Tana Valley Times, "The wrong number" (split.html?f=tana).
// Use cases: one correction that reaches everywhere (the story, the corrections page, the desks'
// answers, AI assistants at the agent door, the Friday five), and answers only from published
// stories. Left: W11's stills (tana_01..12), fictional people made with AI. Right: the real
// sample paper and onboarding pages (right/tana/), and panels drawn here for the old site, the
// assistant on Wanjiku's phone, a traders' chat and a letter.
const phoneShot = src => `<div class="phonebg"><div class="phone"><div class="screen"><img src="${src}" alt=""></div></div></div>`;
const OLD_STORY = note => `
<div class="oldsite">
  <div class="mast">Tana Valley Times<small>The valley's weekly paper</small></div>
  <div class="nav"><span>News</span><span>Council</span><span>Farming</span><span>Business</span><span>Contact</span></div>
  <div class="main">
    <h1>Council approves northern rail extension</h1>
    <p class="meta">By Amani Otieno · Thursday 5 November · 4 comments</p>
    <p>Councillors voted 31 to 12 to approve the first phase of the commuter rail extension, which will cost <mark>KSh 48bn</mark>.</p>
    <p>Phase one covers 14 km of track, four stations and a depot at Kora Junction.</p>
    ${note ? `<p class="note" data-at="1.2">${note}</p>` : `<p>Construction tenders are expected to open in January.</p>`}
  </div>
</div>`;

window.FILMDEF = {
  id: "tana",
  title: "The wrong number",
  sample: "The Tana Valley Times is a sample paper.",
  where: "Tana Valley Times newsroom",
  places: { t02: "Kora market", t03: "A matatu into town", t05: "Kora market", t08: "A matatu home", t09: "Kora market", t10: "A matatu into town", t11: "Mzee Otieno's kitchen" },
  night: ["t06"],
  rlabel: { before: "The paper's website today", after: "The paper's website on Agentrix" },
  stills: k => [`../../agentrix-media/samples/agentrix-home-tana/tana_${k.slice(1)}.webp`, `stills/tana/${k}.webp`],
  shots: {
    t01: "Thursday dawn: three people make the whole paper.", t02: "Mzee Otieno reads the paper at his stall.", t03: "Wanjiku asks her phone on the matatu.",
    t04: "The council calls. Daniel takes it.", t05: "At the market, nobody agrees on the number.", t06: "Late night. Nduku and her laptop.",
    t07: "Same Thursday. Nduku corrects it once.", t08: "Wanjiku reads the right number.", t09: "Mzee Otieno, satisfied.",
    t10: "Friday 7 a.m. Catch me up on the matatu.", t11: "The Friday five over breakfast.", t12: "Months later: a bigger newsroom.",
  },
  images: Object.fromEntries(["site", "story", "corrections", "digest", "agents", "assess-start", "assess", "move"].map(k => [k, `right/tana/${k}.webp`])),
  templates: {
    posts: `
<div class="wp">
  <div class="bar"><span class="home">Tana Valley Times</span><span>+ New</span><span class="spacer"></span><span>Howdy, editor</span></div>
  <nav class="side"><div>Dashboard</div><div class="on">Posts</div><div>Media</div><div>Pages</div><div>Comments <em>38</em></div><div>Plugins <em>4</em></div><div>Settings</div></nav>
  <main>
    <h2>Posts</h2>
    <div class="notice">4 plugins have updates waiting.</div>
    <div class="box"><table>
      <tr class="unread"><td>Council approves northern rail extension</td><td>Amani Otieno</td><td class="status">Published</td></tr>
      <tr><td>Riverside ward clean-up draws 200 volunteers</td><td>Lea Mwangi</td><td class="status">Published</td></tr>
      <tr><td>Maize buyers offer less than last season</td><td>Zawadi Chege</td><td class="status">Published</td></tr>
      <tr><td>Kivuli Primary opens its new library block</td><td>Lea Mwangi</td><td class="status">Draft</td></tr>
      <tr><td>Short rains arrive early across the valley</td><td>Nduku</td><td class="status">Draft</td></tr>
    </table></div>
  </main>
</div>`,
    oldstory: OLD_STORY(""),
    "oldstory-update": OLD_STORY("Update, 5:12 p.m.: the council says the figure for phase one is KSh 41bn."),
    "ai-before": `
<div class="phonebg"><div class="phone"><div class="screen"><div class="ai">
  <div class="head">Assistant<span>on Wanjiku's phone</span></div>
  <div class="msgs">
    <div class="u" data-at="0.3">How much will the northern rail extension cost?</div>
    <div class="a" data-at="2">Phase one will cost <b>KSh 48bn</b>, according to the Tana Valley Times.<small>From a story published Thursday 5 November</small></div>
  </div>
</div></div></div></div>`,
    "ai-after": `
<div class="phonebg"><div class="phone"><div class="screen"><div class="ai">
  <div class="head">Assistant<span>on Wanjiku's phone</span></div>
  <div class="msgs">
    <div class="u" data-at="0.3">How much will the northern rail extension cost?</div>
    <div class="a" data-at="1.8">I asked the Tana Valley Times through its agent door.<br><br>Phase one: <b>KSh 41bn</b>. The paper corrected an earlier figure of KSh 48bn on 5 November.
      <div class="src"><b>Council approves first phase of the northern rail extension</b><span>Tana Valley Times · corrected Thu 5 Nov</span></div></div>
  </div>
</div></div></div></div>`,
    chat: `
<div class="chat">
  <div class="head">Kora market traders<span>112 members</span></div>
  <div class="msgs">
    <div class="m" data-at="0.2"><b>Peter</b>Forwarded: Rail extension will cost KSh 48bn!!</div>
    <div class="m" data-at="1.6"><b>Mama Njeri</b>My niece says the paper now says 41?</div>
    <div class="m" data-at="3"><b>Otieno</b>Which is it? I don't trust either now.</div>
  </div>
</div>`,
    "ask-phone": phoneShot("right/tana/ask-phone.webp"),
    "catch-phone": phoneShot("right/tana/catch-phone.webp"),
    letter: `
<div class="panel"><h3>Letters</h3>
  <div class="card"><div class="who"><i style="background:#9c3d1c">O</i>Mzee Otieno, Kora market</div>
  <p class="quote">You got a number wrong, and you fixed it the same day, everywhere it showed. That's why I pay for the Times.</p></div>
</div>`,
  },
  film: [
    { at: 0, len: 8, mood: "before", clock: "Thursday · 6:00 a.m.", left: "t01", right: { kind: "tpl", key: "posts" }, cap: "Thursday at the Tana Valley Times. Three people make the whole paper." },
    { at: 8, len: 8, mood: "before", clock: "Thursday · 9:00 a.m.", left: "t02", right: { kind: "tpl", key: "oldstory" }, cap: "The rail story is out. One number in it is wrong: KSh 48bn." },
    { at: 16, len: 8, mood: "before", clock: "Thursday · 1:00 p.m.", left: "t03", right: { kind: "tpl", key: "ai-before" }, cap: "AI assistants read it, and repeat it." },
    { at: 24, len: 8, mood: "before", clock: "Thursday · 5:00 p.m.", left: "t04", right: { kind: "tpl", key: "oldstory-update" }, cap: "The council calls. The fix goes at the bottom of the story." },
    { at: 32, len: 7, mood: "before", clock: "Friday · 8:00 a.m.", left: "t05", right: { kind: "tpl", key: "chat" }, cap: "The wrong number keeps travelling. Readers stop trusting either." },
    { at: 39, len: 12, mood: "move", clock: "Friday · 10:00 p.m.", left: "t06", right: { kind: "seq", items: [{ key: "assess-start", from: 0, to: 2 }, { key: "assess", from: 2, to: 5.5 }, { key: "move", from: 5.5, to: 12 }] }, cap: [[0, 5.5, "That night, Nduku found Agentrix. A free check-up first."], [5.5, 12, "Then the Move. The archive comes across, page by page."]] },
    { at: 51, len: 6, mood: "after", clock: "Thursday · 5:00 p.m.", left: "t07", right: { kind: "img", key: "story", zoom: [1, 1.06], origin: "50% 20%" }, title: "Same Thursday.", cap: "" },
    { at: 57, len: 8, mood: "after", clock: "Thursday · 5:02 p.m.", left: "t07", right: { kind: "img", key: "corrections", zoom: [1, 1.1], origin: "30% 30%" }, cap: "Nduku corrects it once. It shows at the top of the story and on the corrections page." },
    { at: 65, len: 8, mood: "after", clock: "Thursday · 5:20 p.m.", left: "t08", right: { kind: "tpl", key: "ask-phone" }, cap: "Readers who ask the site get the corrected figure, quoted from the paper." },
    { at: 73, len: 9, mood: "after", clock: "Thursday · 6:00 p.m.", left: "t09", right: { kind: "seq", items: [{ key: "ai-after", from: 0, to: 5 }, { key: "agents", from: 5, to: 9 }] }, cap: [[0, 5, "AI assistants ask the paper itself, through its agent door."], [5, 9, "They get the corrected figure, and the link to the story."]] },
    { at: 82, len: 8, mood: "after", clock: "Friday · 7:00 a.m.", left: "t10", right: { kind: "tpl", key: "catch-phone" }, cap: "Catch me up: the week in a minute, from the paper's own stories." },
    { at: 90, len: 7, mood: "after", clock: "Friday · 7:00 a.m.", left: "t11", right: { kind: "img", key: "digest", zoom: [1, 1.08], origin: "40% 20%" }, cap: "Every Friday at seven: five stories by email, corrections included." },
    { at: 97, len: 8, mood: "after", clock: "Months later", left: "t12", right: { kind: "tpl", key: "letter" }, cap: "The Tana Valley Times: a number its readers trust." },
    { at: 105, len: 8, mood: "end", end: true },
  ],
};
