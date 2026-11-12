/* Agentrix control-plane concepts: shared shell and interactions.
   Builds the scoped OS shell (status bar, dock, command bar), runs approvals (passkey first, an authenticator-code step-up without one; one or two approvers) and replays terminals. */
(function () {
  const svg = (p) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${p}</svg>`;
  const I = {
    today: svg('<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>'),
    decisions: svg('<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.5 5.1 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.5-6.9A2 2 0 0 0 16.8 4H7.2a2 2 0 0 0-1.7 1.1z"/>'),
    fleet: svg('<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>'),
    rollouts: svg('<path d="m12 3 9 5-9 5-9-5 9-5z"/><path d="m3 13 9 5 9-5"/>'),
    incidents: svg('<path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"/><path d="M12 9v4M12 17h.01"/>'),
    customers: svg('<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8"/>'),
    money: svg('<rect x="2" y="6" width="20" height="12" rx="2"/><circle cx="12" cy="12" r="2.5"/><path d="M6 12h.01M18 12h.01"/>'),
    partners: svg('<path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"/>'),
    trust: svg('<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>'),
    clients: svg('<rect x="3" y="7" width="18" height="13" rx="2"/><path d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>'),
    sites: svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/>'),
    team: svg('<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8"/>'),
  };
  I.billing = I.money;

  const lock = svg('<rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/>');
  const shield = svg('<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/><path d="M9 12l2 2 4-4"/>');

  /* One shell, three scopes (07 §6): the scope chip is the switcher, navigation is the dock,
     decisions are the lane, the scope's Chief WebAgent is the command bar. */
  const SCOPES = {
    op: {
      chip: ['op', 'Z', 'Zavora Cloud', 'Operator · 4 cells · 612 sites'],
      tag: '<span class="scope-tag op">Operator · Zavora Cloud</span>',
      pills: `<span class="pill hide-sm" title="Edge error budget left, 30 days"><span class="hide-md">Error budget</span> <b>58%</b><span class="meter-mini"><span style="width:58%;background:linear-gradient(90deg,#12865a,#2bb37d)"></span></span></span>` +
        `<span class="pill hide-sm" title="Primary on call">On call <b>Achieng O.</b></span>` +
        `<a class="pill trust" href="fleet.html#ledger" title="Operator ledger">${shield}<span class="hide-sm">Ledger matched</span><span class="n" aria-label="decisions waiting">3</span></a>` +
        `<button class="pill hold" type="button" title="Platform hold: a separate switch from an owner's freeze; two people for the fleet">${lock}<span class="hide-sm">Platform hold</span></button>`,
      apps: [['fleet', 'Fleet', 'fleet.html'], ['rollouts', 'Rollouts', 'rollout.html'], ['incidents', 'Incidents', 'incident.html', 1],
        ['customers', 'Customers', 'fleet.html#customers'], ['money', 'Money', 'billing.html'], ['partners', 'Partners', 'portfolio.html'], ['trust', 'Trust', 'fleet.html#ledger']],
      agents: [['CW', 'Chief', 'av-op-coord', '#4a4690', 'working', 'Operator Chief WebAgent: sent the ops brief'],
        ['Sr', 'SRE', 'av-sre', '#d0583f', 'listening', 'SRE: watching every cell'],
        ['Re', 'Release', 'av-release', '#6b8cf0', 'waiting', 'Release: waiting for you'],
        ['Cp', 'Capacity', 'av-capacity', '#2aa6c9', 'waiting', 'Capacity: waiting for you'],
        ['Bi', 'Billing', 'av-billing', '#2fb07a', 'waiting', 'Billing: waiting for you'],
        ['Su', 'Support', 'av-support', '#e3607b', 'answering', 'Support: answering tickets'],
        ['Sc', 'Security', 'av-security', '#45597f', 'listening', 'Security & Abuse: watching the abuse desk'],
        ['Ps', 'Partners', 'av-partner', '#22a394', 'working', 'Partner Success: planning an import'],
        ['Gr', 'Growth', 'av-growth', '#c58016', 'asleep', 'Growth: asleep until 10:00'],
        ['Cm', 'Compliance', 'av-compliance', '#a35dc1', 'asleep', 'Compliance: asleep until Friday']],
      ask: 'Ask the Operator Chief WebAgent… e.g. which partners are importing next week?',
    },
    pf: {
      chip: ['pf', 'M', 'Studio Mwangaza', 'Portfolio · 24 clients · 31 sites'],
      tag: '<span class="scope-tag pf">Portfolio · on Agentrix</span>',
      pills: `<span class="pill hide-sm" title="WordPress import">Import <b>14 / 40</b><span class="meter-mini"><span style="width:35%"></span></span></span>` +
        `<a class="pill trust" href="portfolio.html#held" title="Client cards">${shield}<span class="hide-sm">Grants in order</span><span class="n" aria-label="decisions waiting">4</span></a>` +
        `<button class="pill freeze" type="button" title="Freeze your clients' sites, over your grants">${lock}<span class="hide-sm">Freeze clients</span></button>`,
      apps: [['clients', 'Clients', 'portfolio.html'], ['sites', 'Sites', 'portfolio.html#import'], ['rollouts', 'Rollouts', 'portfolio.html#ring'],
        ['billing', 'Billing', 'portfolio.html#margin'], ['team', 'Team', 'portfolio.html#brand']],
      agents: [['CW', 'Chief', 'av-chief', '#5c58a6', 'working', 'Your Chief WebAgent: portfolio brief sent'],
        ['Re', 'Release', 'av-release', '#6b8cf0', 'working', 'Release: theme ring in progress'],
        ['Su', 'Support', 'av-support', '#e3607b', 'answering', 'Support: answering clients'],
        ['Bi', 'Billing', 'av-billing', '#2fb07a', 'asleep', 'Billing: asleep until month end']],
      ask: 'Ask your Chief WebAgent… e.g. which clients need me this week?',
    },
  };

  document.querySelectorAll('[data-cp-scope]').forEach((os) => {
    const s = SCOPES[os.dataset.cpScope];
    const current = os.dataset.app;
    const bar = os.querySelector('[data-cp-bar]');
    const dock = os.querySelector('[data-cp-dock]');
    const cmd = os.querySelector('[data-cp-cmd]');
    const [cls, mark, name, sub] = s.chip;
    if (bar) bar.innerHTML =
      `<button class="site-chip scope-chip ${cls}" type="button" aria-label="Switch scope: Site, Portfolio or Operator"><span class="logo-bake">${mark}</span><span>${name}<small>${sub}</small></span><span class="caret" aria-hidden="true">▾</span></button>` +
      s.tag + `<div class="bar-right">${s.pills}</div>`;
    if (dock) {
      dock.setAttribute('aria-label', name + ': apps and WebAgents');
      dock.innerHTML =
        s.apps.map(([id, label, href, badge]) =>
          `<a class="app-ico" href="${href}"${id === current ? ' aria-current="true"' : ''} title="${label}"><span class="sys">${I[id]}</span>${label}${badge ? `<span class="badge">${badge}</span>` : ''}</a>`).join('') +
        '<span class="sep" aria-hidden="true"></span>' +
        s.agents.map(([ini, label, av, c, state, title]) =>
          `<button class="app-ico" type="button" title="${title}"><span class="os-mark" data-state="${state}" style="--c:${c}"><span class="avatar ${av}">${ini}</span></span>${label}</button>`).join('');
    }
    if (cmd) cmd.innerHTML =
      `<div class="askline"><span class="orb sm" aria-hidden="true"></span><input type="text" aria-label="${s.ask.split('…')[0]}" placeholder="${s.ask}">` +
      `<button class="icon-btn" type="button" aria-label="Send">${svg('<path d="M5 12h14M13 6l6 6-6 6"/>')}</button></div>`;
  });

  /* Approvals (02 §8): a passkey first; without one, an MFA step-up, a single-use authenticator
     code bound to the card's fingerprint. The completion line names the method, as the receipt
     does. A card with two pending signers waits for a simulated second approver; two-person rules
     stay two people whichever method each uses. */
  const byCode = (t) => t
    .replace(/signed with (\S+?)'s passkey/g, 'by $1 with an authenticator code')
    .replace(/signed on (\S+?)'s phone/g, "with an authenticator code on $1's phone")
    .replace(/signed by /g, 'by ')
    .replace(/(\d) of (\d) signed/g, '$1 of $2 approved')
    .replace(/Your signature is recorded/g, 'Your approval is recorded');
  const settle = (card, method) => {
    const line = card.querySelector('.done-line');
    const say = (t) => (method === 'code' ? `${byCode(t)} Your method in the receipt: authenticator code (MFA step-up).` : t);
    card.classList.remove('signing', 'stepping');
    const pending = [...card.querySelectorAll('.signer:not(.signed)')];
    if (pending[0]) pending[0].classList.add('signed');
    card.classList.add('done');
    if (pending.length > 1 && card.dataset.partial) {
      line.textContent = say(card.dataset.partial);
      setTimeout(() => {
        pending[1].classList.add('signed');
        line.textContent = say(card.dataset.done);
      }, 2400);
    } else if (card.dataset.done) {
      line.textContent = say(card.dataset.done);
    }
  };
  document.querySelectorAll('.decision').forEach((card) => {
    const approve = card.querySelector('[data-approve]');
    const sheet = card.querySelector('.sheet');
    const actions = card.querySelector('.actions');
    if (!approve || !sheet || !actions) return;
    const passkeySheet = sheet.innerHTML;
    approve.addEventListener('click', () => {
      sheet.innerHTML = passkeySheet;
      card.classList.add('signing');
      setTimeout(() => settle(card, 'passkey'), 1300);
    });
    const alt = document.createElement('button');
    alt.type = 'button';
    alt.className = 'stepup-link';
    alt.textContent = 'No passkey on this device? Use your authenticator code';
    actions.after(alt);
    alt.addEventListener('click', () => {
      sheet.innerHTML = '<b>Enter the code from your authenticator app</b>' +
        '<div class="code-cells" aria-hidden="true"><span>4</span><span>8</span><span>2</span><span>9</span><span>1</span><span>7</span></div>' +
        '<div class="small" style="opacity:.8">Single use · bound to this card\'s exact effect · the receipt says "authenticator code"</div>';
      card.classList.add('signing', 'stepping');
      setTimeout(() => settle(card, 'code'), 1900);
    });
  });
  document.querySelectorAll('.decision [data-decline]').forEach((btn) => {
    btn.addEventListener('click', () => {
      const card = btn.closest('.decision');
      card.classList.add('done', 'declined');
      card.querySelector('.done-line').textContent = card.dataset.declined || 'Declined. Nothing happened. The reason is recorded.';
    });
  });

  /* "Send to client": an agency forwards an owner-only card to the owner's passkey (07 §6). */
  document.querySelectorAll('[data-send]').forEach((btn) => {
    btn.addEventListener('click', () => btn.closest('.ldec').classList.add('done'));
  });

  /* Terminal replay */
  const reduce = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  document.querySelectorAll('.reveal').forEach((root) => {
    const items = [...root.querySelectorAll(':scope > .l, :scope > .box')];
    let timer;
    const play = () => {
      clearTimeout(timer);
      items.forEach((el) => el.classList.remove('shown'));
      if (reduce || document.hidden) { items.forEach((el) => el.classList.add('shown')); return; }
      let i = 0;
      const step = () => {
        if (i >= items.length) return;
        const el = items[i++];
        el.classList.add('shown');
        timer = setTimeout(step, Number(el.dataset.wait || (el.classList.contains('box') ? 650 : 170)));
      };
      step();
    };
    const replay = document.querySelector(`[data-replay="${root.id}"]`);
    if (replay) replay.addEventListener('click', play);
    play();
  });
})();
