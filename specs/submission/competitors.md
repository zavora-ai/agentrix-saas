# W14: Agentrix competitors as of November 2026 (fast pass)

Researched 2026-11-08 in about 10 minutes, using vendor pages where they could be reached and coverage otherwise. Numbers in [brackets] point to the sources below. Where a source is a third-party blog, the cell says so. If a feature could not be confirmed, it is marked unknown and the placement is kept low.

Columns: (a) AI acts for the business 24/7 (answers, orders, bookings); (b) features for customers' AI agents; (c) owner approves consequential AI actions; (d) moving in an existing site, especially cleaning a hacked one; (e) cheapest paid plan (USD/mo).

| Competitor | (a) 24/7 AI for the business | (b) Customers' agents | (c) Owner approval | (d) Move-in / hacked clean | (e) Cheapest paid | X | Y |
|---|---|---|---|---|---|---|---|
| **Wix** | **Partial.** AI Site Chat answers visitors around the clock and helps them make purchases. Smart Chat replaced it for new sites in 2026; it doesn't book on its own [2] | **Yes.** Every Wix site has a built-in Site MCP: visitors' agents can book services and start a purchase [1]. llms.txt API [4] | **Unknown.** The agent acts as an anonymous visitor; no owner approval step is described [1] | **Unknown.** No hacked-site cleaning found | Light $17/mo annual, reported $19 since Aug 2026 [3] | **6** | **2** |
| **WordPress.com / Automattic** | **No (none found).** MCP and Jetpack AI help the owner run content; no native agent serving customers was found [5] | **Partial.** The MCP server (read Oct 2025, write Mar 2026) is for the owner's own AI clients, not customers' agents [5][6] | **Yes**, for the owner's agents: "Approve every change, approve nothing, or anything in between"; new posts start as drafts [5] | **No.** Jetpack says Scan "is not intended" to clean up already-hacked sites [7] | Personal $4/mo annual ($9 monthly) [8] | **3** | **5** |
| **Self-hosted WordPress + plugins** | **Partial**, only through third-party chat plugins (e.g. Tidio, below) | **Partial.** Abilities API in 6.9, a separate MCP Adapter plugin, and llms.txt plugins [9] | **Unknown.** Depends on the plugin; core gives role permissions only [9] | **Partial.** Manual cleanup or paid security services; no clean-on-move [7] | Free software; hosting varies | **3** | **1** |
| **Shopify** | **Partial.** The free Inbox app "puts an AI-powered sales associate on your storefront"; its listing doesn't say it takes orders [12] | **Yes.** Storefront/Global Catalog, Cart, Checkout and Order MCP on UCP; "trusted agents can complete checkouts directly" [10]. Agentic Storefronts reach ChatGPT and Copilot [11]. Native llms.txt (third-party report) [13] | **Partial.** Sidekick shows changes for merchant review before applying them (third-party summaries) [14]. Agent checkouts are gated by Shopify's trust tiers, not by the owner [10] | **Unknown.** No hacked-site cleaning (it's a rebuild, not a move) | Basic $29/mo annual ($39 monthly). Starter $5 reportedly closed to new US stores [15] | **7** | **4** |
| **Squarespace** | **No.** Blueprint AI is a site builder [17]; no native visitor chatbot confirmed (third-party chatbots go in through code injection) | **Partial.** Native llms.txt on 7.1 sites (2026) [16]; no MCP found | **Unknown** | **Unknown** | Basic $19/mo annual ($25 monthly) after the July 2026 increase [18] | **1** | **1** |
| **Webflow** | **No.** No customer-facing AI agent found | **Partial.** llms.txt support since Apr 2025 [20]. The MCP server is for builders' agents, not visitors' [19] | **Partial.** MCP 2.0 (Jul 2026): roles per site, page and locale, "human-or-AI attribution" logs, branch-isolated edits; no approval workflow mentioned [19] | **Unknown** | Basic $15/mo annual, after the May 2026 restructure [21] | **2** | **5** |
| **GoDaddy Airo** | **Unknown.** Airo.ai agents do owner tasks (site, domains). The Conversations Inbox agent gathers customer messages in one place; no confirmed auto-reply [22] | **Partial.** Agent Name Service gives AI agents verifiable DNS/PKI identity, but it isn't an agent door on small-business sites [23] | **Unknown** | **Partial.** Website Security plans include malware cleanup (from $6.99/mo), not tied to migration [24] | Airo Starter ~$9.99/mo annual (third-party) [25] | **3** | **3** |
| **Hostinger** | **No.** Hostinger Agent (replaced Kodee in Aug 2026) acts "across your Hostinger account" for the owner, not for site visitors [26][27] | **Unknown** | **Partial.** Deploys "after confirmation"; advises owners to review before publishing [26] | **Partial.** Free migration; malware scanner (files only); paid WordPress cleanup ($95) [28] | ~$2.99/mo on a 48-month promo ($11.99 month-to-month) [29] | **3** | **3** |
| **Durable** | **Partial.** "Capture & reply to leads automatically", "Get booked online, 24/7", AI chat reply; "not designed" for e-commerce [30] | **Partial.** "llms.txt for AI assistants" [30]; no MCP | **Unknown** | **Unknown.** No import mentioned [30] | Launch $22/mo annual ($25 monthly) [30] | **4** | **1** |
| *10Web (opt.)* | **Unknown** | **Unknown.** No MCP found [31] | **Unknown** | **Partial.** AI "recreates" an existing site in WordPress; no cleaning claim [31] | ~$10/mo annual [31] | **1** | **1** |
| *Intercom Fin (opt.)* | **Partial.** A 24/7 AI support agent, billed per resolution; it's a chat add-on, not the site [32] | **Unknown** | **Unknown** | n/a | $0.99/resolution plus seats from $29 [32] | **5** | **2** |
| *Tidio Lyro (opt.)* | **Partial.** Answers, checks order status, books through Calendly, handles refunds (third-party) [33] | **Unknown** | **Unknown.** No approval step found [33] | n/a | Starter $29; Lyro from $39/mo (third-party) [33] | **5** | **1** |

**Why each sits where it does (X = site works for the business; Y = governance and containment)**
- **Wix (6, 2):** It is the only builder whose sites all carry an agent door (Site MCP) and a 24/7 chat, but nothing gates what the AI does beyond visitor-level access.
- **WordPress.com (3, 5):** Owner-approved, draft-first actions for the owner's agents earn a mid Y. No AI serves customers, so X stays low.
- **Self-hosted WP (3, 1):** Plugins can add chat and MCP. There is no built-in governance, and hacked-site cleanup is a manual job.
- **Shopify (7, 4):** It has the strongest commerce agent door (MCP, UCP, checkout) and an Inbox AI. Sidekick asks for review, but customer-side actions follow Shopify's trust tiers, not the owner.
- **Squarespace (1, 1):** llms.txt is the only agent-facing feature found. The AI builds the site and then stops.
- **Webflow (2, 5):** It has the best AI governance among builders (roles, attribution logs, branches), but it governs builders' agents only. Nothing acts for customers.
- **GoDaddy Airo (3, 3):** Agentic help is aimed at the owner. ANS identity work and paid malware cleanup lift Y slightly. Customer-facing AI isn't confirmed.
- **Hostinger (3, 3):** "Runs the business" marketing, but the agent works on the owner's account. Confirmation before deploy, migration and file scanning are each partial.
- **Durable (4, 1):** It claims 24/7 lead replies and bookings plus llms.txt. No governance was found.
- **10Web (1, 1):** An AI rebuild of WordPress sites; no agent or governance features found.
- **Intercom Fin and Tidio (5, 2 and 5, 1):** They answer customers 24/7 and can take some actions. They are bolt-on chat, with no agent door and no owner-approval trail found.
- *(Not researched; for the slide's reference only: Agentrix's own claim would sit around X 9, Y 9 because of the passkey approvals and sealed sandboxes. That is our reading, not a measurement.)*

**Caveats:** Pricing pages moved during 2026 (Wix, Squarespace, Webflow and Shopify Starter all changed), so prices are as reported. Shopify's Sidekick approval behaviour comes from third-party summaries, not Shopify's own docs. No competitor was found that replays and cleans a hacked site while moving it.

## Sources
1. Wix Site MCP: https://dev.wix.com/docs/develop-websites-sdk/get-started/ai-agents/about-the-wix-site-mcp
2. Wix AI Site Chat (being replaced by Smart Chat): https://support.wix.com/en/article/about-the-wix-ai-chat-assistant
3. Wix Light price: https://www.stylefactoryproductions.com/blog/wix-pricing ; price-rise tracker: https://www.usagepricing.com/blueprint/activity/wix-2026-08-27-light-plan-price-rise
4. Wix llms.txt API: https://dev.wix.com/docs/api-reference/business-management/marketing/seo/txt-file-server/llms-txt/introduction
5. WordPress.com, AI agents can write and manage content (20 Mar 2026): https://wordpress.com/blog/2026/03/20/ai-agent-manage-content/
6. CMSWire on WordPress.com MCP write access: https://cmswire.com/digital-experience/wordpresscom-enables-ai-agents-to-write-manage-content
7. Jetpack, cleaning a hacked WordPress site: https://jetpack.com/support/scan/how-to-clean-your-hacked-wordpress-site/
8. WordPress.com plans (Oct 2026): https://eseospace.com/blog/wordpress-com-plans/
9. WordPress Abilities API and MCP Adapter: https://jorijn.com/en/blog/wordpress-7-0-abilities-mcp-plugin-developers-learn-first/
10. Shopify for agents (Catalog, Cart, Checkout and Order MCP, UCP): https://shopify.dev/docs/agents
11. Shopify Agentic Storefronts: https://www.shopify.com/news/winter-26-edition-agentic-storefronts
12. Shopify Inbox app listing: https://apps.shopify.com/inbox
13. Shopify native llms.txt (third-party): https://nikhil.pro/shopify-quietly-rolled-out-agentic-commerce-and-llms-txt-on-every-store/
14. Sidekick merchant approval (third-party Fall '26 summary): https://coalitiontechnologies.com/blog/shopify-editions-fall-26
15. Shopify pricing: https://www.nerdwallet.com/article/small-business/shopify-pricing ; Starter closed: https://getkanal.com/blog/shopify-starter-plan
16. Squarespace native llms.txt: https://www.collaborada.com/blog/squarespace-llms-txt
17. Squarespace AI tools: https://www.squarespace.com/blog/ai-tools-overview
18. Squarespace pricing: https://www.activepieces.com/blog/squarespace-pricing-in-2026-and-what-a-site-costs.md ; https://petapixel.com/2026/07/17/squarespace-is-increasing-prices-by-up-to-26/
19. Webflow MCP 2.0: https://www.cmswire.com/digital-experience/webflow-mcp-20-brings-governance-to-ai-site-building/ ; https://webflow.com/blog/mcp-2-features
20. Webflow MCP server and llms.txt: https://developers.webflow.com/data/changelog/04082025-mcp-server
21. Webflow pricing 2026: https://www.digidop.com/blog/webflow-site-pricing
22. GoDaddy Airo.ai launch: https://www.3blmedia.com/news/godaddy-brings-agentic-ai-small-businesses-launch-airoai ; six more agents: https://martechvibe.com/article/godaddy-adds-six-new-ai-agents-to-expand-airo-ai-capabilities/
23. GoDaddy Agent Name Service: https://www.godaddy.com/resources/news/building-the-open-agentic-web-why-identity-and-discovery-must-work-together
24. GoDaddy Website Security plans: https://www.godaddy.com/web-security/website-security/selectplan
25. Airo pricing (third-party): https://hostadvice.com/hosting-company/godaddy-reviews/airo-pricing/
26. Hostinger Agent: https://www.hostinger.com/support/hostinger-agent/
27. Hostinger AI Builder (Aug 2026): https://techradar.com/pro/its-not-about-who-can-generate-a-homepage-in-30-seconds-hostinger-launches-a-new-agentic-ai-builder-helping-businesses-do-more-with-ai
28. Hostinger migration: https://www.hostinger.com/support/4455931-how-can-i-migrate-website-to-hostinger/ ; malware scanner: https://www.hostinger.com/support/6197555-how-to-use-the-malware-scanner-at-hostinger/
29. Hostinger pricing: https://www.websitebuilderexpert.com/hosting-websites/hostinger-pricing/
30. Durable pricing and features: https://durable.com/pricing
31. 10Web: https://toolradar.com/tools/10web/pricing ; https://elementor.com/blog/10web-alternatives/
32. Fin (Intercom) pricing: https://sitegpt.ai/blog/intercom-fin-ai-review
33. Tidio Lyro: https://getkanal.com/blog/tidio-pricing
