#!/usr/bin/env python3
"""Writes the Tana Valley Times sample bundle (01a §1.6, D2, D4; 03-data §5.5 adapted to a
local weekly of about 60 stories).

Run from the repository root: python3 fixtures/samples/tools/tana.py
The paper, its editor, reporters, readers, places, councillors and figures are invented. Every
email is at example.com. Times are relative to the load instant (agentrix-records/src/time.rs),
drawn for a load on Sunday 8 November 2026, so "d-3 11:04" is Thursday 5 November.
"""
import json
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "tana-valley-times")


def write(name, rows):
    path = os.path.join(OUT, "records", f"{name}.jsonl")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")


SECTIONS = [
    {"id": "transport", "name": "Transport", "about": "Roads, bridges, matatu routes and the northern rail extension. Report a road problem to the newsroom by email; the Times passes reports to the county roads office and follows up in print."},
    {"id": "council", "name": "Council", "about": "What the county council voted on, spent and decided. Council sittings are open to the public at the county hall in Mto Mkuu, and the Times publishes how every ward's councillor voted."},
    {"id": "farming", "name": "Farming and weather", "about": "Rain, harvests, crop prices and advice from extension officers. Rain figures come from the county's own gauges."},
    {"id": "business", "name": "Business", "about": "Markets, shops, jobs and money in the valley. Traders can send their prices to the newsroom each week; the Times prints the figures it can check."},
    {"id": "community", "name": "Community", "about": "Schools, clinics, sport and the people of the valley. Send news of your school, clinic, church or club to the newsroom by email; the Times prints what it can confirm."},
]

manifest = {
    "site": "tana-valley-times-sample",
    "host": "tana-valley-times-sample.agentrix.website",
    "name": "Tana Valley Times",
    "archetype": "news",
    "currency": "KES",
    "utc_offset_min": 180,
    "seed_version": "2026-11-08.7",
    "facts": {
        "order_prefix": "TV",
        "domain": "tana-valley-times.example",
        "since": "2006",
        "tagline": "The valley's weekly paper, online every day",
        "owner": {"login": {"email": "nduku@tana-valley-times.example", "password": "tana-sample-editor"}, "name": "Nduku Mwikali", "first": "Nduku", "initial": "N", "role": "Editor", "channel": "email", "wait": "a day",
                  "email": "editor@tana-valley-times.example"},
        "release": "v3",
        "mark": {"family": "strata", "squareness": 7.2, "key": "tana", "font": "Libre Caslon Text"},
        "front_color": "#14213d",
        "front": {"lead": "council-approves-first-phase-of-the-northern-rail-extension", "topic": "rail"},
        "sections": SECTIONS,
        "topics": {
            "rail": {"title": "The northern rail extension", "phrase": "the northern rail extension", "about": "The commuter line from Mto Mkuu to Kivuli, from the first study to the tenders."},
            "budget": {"title": "The county budget", "phrase": "the county budget", "about": "How the council raises and spends the county's money, year by year."},
            "market": {"title": "Ndovu market's move", "phrase": "Ndovu market's move", "about": "The traders of Ndovu market and the plan to move them to a new site."},
        },
        "house_rules": [
            "Desks never write news. They answer from published reporting.",
            "Every claim links to a dated, bylined story.",
            "Subscriber stories: headline and link only, unless you subscribe.",
            "Corrections reach answers and digests within minutes.",
            "The editor approves each desk's scope and wording.",
        ],
        "digest": {
            "name": "Friday five",
            "day": "Friday",
            "time": "07:00",
            "count": 5,
            "contract": [
                ["You get", "Every Friday at 07:00: five {topics} stories, two lines each, with links"],
                ["Made from", "Only stories Tana Valley Times published that week. The two lines are each story's own summary, as the newsroom wrote it"],
                ["We keep", "Your email and these topics. Not your reading history"],
                ["Stop", "One tap in any email. Your topics are deleted with it"],
            ],
        },
        "subscription": {
            "plan": "Supporter",
            "price_words": "KSh 300 a month (sample price)",
            "price": 300,
            "period": "month",
            "days": 30,
            "usd_rate": 129.0,
            "note": "KSh 300 is a sample price until the editor sets the real one",
            "includes": ["Every subscriber story in full, including the archive"],
            "renewal": "This sample takes one payment for one month. It doesn't renew by itself; pay again to keep reading.",
        },
        "device_memory": {
            "items": ["last_visit", "follows"],
            "sample": {"last_visit_days_ago": 5, "follows": ["Transport"]},
        },
        "desks": [
            {"id": "newsroom", "name": "Newsroom", "href": "/", "color": "#b3261e", "glyph": "news", "seed": 21,
             "mandate": "I answer from Tana Valley Times' published stories and link every one, with its byline and date. I never write news.",
             "starters": [["Where does the rail extension stand?", "standing", True], ["Who has been reporting it?", "who", True], ["Is the extension a good idea?", "opinion", True]]},
            {"id": "archive", "name": "Archive", "href": "/archive", "color": "#2f5d8a", "glyph": "calendar", "seed": 9,
             "mandate": "I find past issues by date and topic, back to the first story online.",
             "starters": [["When did the rail story start?", "first", True], ["What did the paper report last month?", "month", True], ["Show me the market stories", "topic", True]]},
            {"id": "subscriptions", "name": "Subscriptions", "href": "/subscribe", "color": "#14213d", "glyph": "coin", "seed": 15,
             "mandate": "I handle subscriptions and the Friday five. I never take card details in a chat.",
             "starters": [["How much is a subscription?", "price", True], ["Is my subscription active?", "status", True], ["How do I stop the Friday five?", "stop", True]]},
        ],
    },
}
os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "manifest.json"), "w") as f:
    json.dump(manifest, f, ensure_ascii=False, indent=2)
    f.write("\n")

write("authors", [
    {"author_id": "au-amani", "name": "Amani Otieno", "role": "Council and transport reporter", "bio": "Amani has covered the county council for the Times since 2019, and the rail plans since the first study."},
    {"author_id": "au-lea", "name": "Lea Mwangi", "role": "Community reporter", "bio": "Lea reports on schools, clinics, sport and the wards along the river."},
    {"author_id": "au-daniel", "name": "Daniel Kiprop", "role": "Business reporter", "bio": "Daniel follows the markets, the banks and what things cost in the valley."},
    {"author_id": "au-zawadi", "name": "Zawadi Chege", "role": "Farming and weather reporter", "bio": "Zawadi reports from the farms, the cooperatives and the rain gauges of the upper valley."},
])

S = []


def story(slug, section, title, standfirst, author, at, paras, kicker=None, topic=None, ent="free", updated=None):
    S.append({"slug": slug, "section": section, "kicker": kicker, "title": title, "standfirst": standfirst, "author": author,
              "published_at": at, "topic": topic, "entitlement": ent, "body": paras, "updated_at": updated})


# ---------------------------------------------------------------- the rail story (topic rail)
story("feasibility-study-backs-a-northern-rail-line", "transport", "Feasibility study backs a northern rail line",
      "A study paid for by the county says a commuter line from Mto Mkuu to Kivuli would carry 18,000 people a day by its fifth year.",
      "au-amani", "d-420 06:00", [
          "A feasibility study commissioned by the county council has found that a commuter rail line from Mto Mkuu to Kivuli would carry about 18,000 passengers a day within five years of opening.",
          "The study looked at three routes. It recommends the eastern route along the old cattle track, which passes Kora Junction and Riverside ward and needs the least land.",
          "The council's transport committee will hold public hearings before any route is chosen, its chair told the Times.",
          "The study puts the cost of the whole line at between KSh 70bn and KSh 85bn, built in two phases.",
      ], topic="rail")
story("public-hearings-on-the-northern-line-route-begin", "transport", "Public hearings on the northern line's route begin",
      "Residents of four wards have six weeks to tell the council where the stations should go.",
      "au-amani", "d-90 06:00", [
          "The first of eight public hearings on the northern rail extension opened at Kora Junction social hall, with about 150 people attending.",
          "Most speakers asked about land. Farmers along the eastern route wanted to know how the county would value their plots and when it would pay.",
          "The hearings continue in Riverside, Mlimani and Kivuli wards. Written comments can be left at the county hall until the last hearing closes.",
      ], topic="rail")
story("council-publishes-the-northern-rail-extension-proposal", "transport", "Council publishes the northern rail extension proposal",
      "The proposal splits the line into two phases and puts four stations in phase one.",
      "au-amani", "d-6 09:00", [
          "The county council has published its proposal for the northern rail extension, ahead of a vote by the full council this week.",
          "Phase one would run 14 km from Mto Mkuu station to Kora Junction, with stops at the hospital, Riverside and Kora market, and a depot at Kora Junction.",
          "Phase two, to Kivuli, would follow once phase one is running. The proposal does not give a date for it.",
          "The document is on the council's notice board at the county hall and can be read at the Mto Mkuu library.",
      ], topic="rail")
story("residents-split-over-the-northern-line-route", "transport", "Residents split over the northern line's route",
      "Riverside wants a station near the market; Kora wants one beside the clinic.",
      "au-lea", "d-4 10:30", [
          "In Riverside ward, many residents want the station closer to the market, where most people work. In Kora, residents want it beside the clinic, so that older people can reach it on foot.",
          "\"We were heard at the hearings, but the map did not change,\" said a Riverside shopkeeper who asked not to be named.",
          "At Kora Junction, the farmers' cooperative supports the depot site because it would bring a tarmac road to its cold store.",
          "The council says station positions can still move by up to 300 metres during the land survey.",
      ], topic="rail")
story("council-approves-first-phase-of-the-northern-rail-extension", "transport", "Council approves first phase of the northern rail extension",
      "Councillors voted 31 to 12 to approve phase one of the commuter rail extension, with construction tenders to open in January.",
      "au-amani", "d-3 11:04", [
          "Councillors voted 31 to 12 at a full sitting in Mto Mkuu to approve phase one of the northern rail extension, the commuter line that would run from Mto Mkuu station towards Kivuli.",
          "Phase one covers 14 km of track, four stations and a depot at Kora Junction. The council's published estimate for this phase is KSh 41bn, to be met from a national transport grant and a county bond.",
          "Construction tenders open in January, the transport committee said.",
          "Councillors who voted against said the route through Riverside ward had not been settled, and asked for more hearings before land is bought.",
          "The committee will publish the tender documents and a timetable for the land survey before the end of the year.",
      ], kicker="Transport", topic="rail", updated="d-3 16:20")

# ---------------------------------------------------------------- the county budget (topic budget)
story("last-year-budget-what-was-spent", "council", "Last year's budget: what was spent",
      "The council spent 87 percent of its development budget, most of it on roads and water.",
      "au-amani", "d-350 06:00", [
          "The county's audited accounts show the council spent 87 percent of the money it set aside for building work last year.",
          "Roads took the largest share, followed by water pipes and the new maternity wing at Mto Mkuu hospital. The ward development fund spent only half of its allocation.",
          "The finance committee said delays in national transfers held back the last quarter's spending.",
      ], topic="budget")
story("council-asks-residents-what-the-budget-should-fund", "council", "Council asks residents what the budget should fund",
      "Ward meetings start next week, and written ideas can be left at any ward office.",
      "au-amani", "d-51 06:00", [
          "The county council has opened its yearly budget consultation, with a meeting in every ward over the next three weeks.",
          "The finance committee says it wants to hear which roads, water points and clinics residents would put first.",
          "Last year the meetings drew about 2,000 people across the county. Ideas can also be written down and left at any ward office.",
      ], topic="budget")
story("county-budget-hearings-start-in-mto-mkuu", "council", "County budget hearings start in Mto Mkuu",
      "Departments defend their spending plans in front of the finance committee for the first time in public.",
      "au-amani", "d-9 10:00", [
          "Heads of the county's departments began presenting their spending plans to the finance committee, in hearings open to the public for the first time.",
          "The roads department asked for the largest increase, citing the rail extension's access roads. The health department asked for money to keep the Mlimani night shift running.",
          "The hearings run for two weeks at the county hall.",
      ], topic="budget")
story("finance-committee-asks-wards-to-cut-spending-requests", "council", "Finance committee asks wards to cut spending requests",
      "Ward requests add up to a third more than the county expects to receive.",
      "au-amani", "d-1 09:15", [
          "The finance committee has asked every ward to trim its spending requests, after they came to about a third more than the county expects to receive next year.",
          "The committee chair said the rail extension's share would not come out of ward money, because it is funded by a national grant and a bond.",
          "Wards have two weeks to send revised lists. The full council votes on the budget next month.",
      ], topic="budget")

# ---------------------------------------------------------------- Ndovu market (topic market)
story("ndovu-market-traders-form-an-association", "business", "Ndovu market's traders form an association",
      "More than 300 stallholders sign up in the first month.",
      "au-daniel", "d-1080 06:00", [
          "Traders at Ndovu market have formed an association to speak for them with the council, after years of disputes over fees and rubbish collection.",
          "More than 300 stallholders signed up in the first month. The association's first request is a roof over the vegetable section.",
      ], topic="market")
story("ndovu-market-fire-traders-count-their-losses", "business", "Ndovu market fire: traders count their losses",
      "About 60 stalls burned overnight. No one was hurt.",
      "au-daniel", "d-510 06:00", [
          "A fire destroyed about 60 stalls in the clothing section of Ndovu market overnight. No one was hurt.",
          "The traders' association estimates its members lost stock worth about KSh 9m. The county fire engine arrived from Mto Mkuu after 40 minutes.",
          "The council has promised temporary stalls within two weeks and a review of where the market should be.",
      ], topic="market")
story("ndovu-market-relocation-plan-published", "business", "Ndovu market relocation plan published",
      "The council wants to move the market to a site with water, drainage and a fire point.",
      "au-daniel", "d-26 06:00", [
          "The county council has published its plan to move Ndovu market to a five-acre site beside the Kanyonga road, with piped water, drains and a fire point.",
          "The plan gives each current stallholder a stall at the new site at the same fee for two years.",
          "The traders' association says it supports the move in principle but wants a written list of who gets which stall.",
      ], topic="market")
story("ndovu-market-traders-get-a-date-for-the-move", "business", "Ndovu market traders get a date for the move",
      "Stallholders will move in February, once the new site's water connection is finished.",
      "au-daniel", "d-2 07:30", [
          "The council has told Ndovu market's traders they will move to the new site in February, once its water connection is finished.",
          "The traders' association and the council have agreed a stall list, which will be posted at the market office this week.",
      ], topic="market", updated="d-2 11:30")

# ---------------------------------------------------------------- this week and last
story("short-rains-arrive-early-across-the-valley", "farming", "Short rains arrive early across the valley",
      "The county's gauges recorded heavy rain a fortnight before the usual start, and more is expected through December.",
      "au-zawadi", "d-5 08:30", [
          "The short rains reached the valley about two weeks earlier than usual, with 112 mm recorded at the Mto Mkuu gauge in four days.",
          "Extension officers advise farmers who have not yet planted beans to do so this week, while the soil is wet.",
          "The county weather office expects above-average rain through December.",
      ], updated="d-5 12:10")
story("clinic-vaccination-day-at-kanyonga", "community", "Vaccination day at Kanyonga clinic set for next week",
      "Children under five can get every routine vaccine in one visit.",
      "au-lea", "d-5 13:00", [
          "Kanyonga clinic will hold a vaccination day next week for children under five, with every routine vaccine available in one visit.",
          "Parents should bring the child's clinic card. The clinic opens at 08:00 and closes when the queue is done.",
      ])
story("what-the-fuel-price-review-means-for-fares", "business", "What the fuel price review means for matatu fares",
      "The monthly fuel price review sets what operators pay. Here is how that reaches the fare you pay to Mto Mkuu.",
      "au-daniel", "d-4 14:00", [
          "Every month the national energy regulator reviews the pump price of diesel. Matatu operators in the valley set their fares from that price, their loan repayments and the cost of spares.",
          "A matatu on the Kanyonga to Mto Mkuu route uses about 40 litres of diesel a day. A rise of KSh 10 a litre adds about KSh 400 to the operator's daily costs.",
          "The valley's matatu association says fares rise when diesel goes up by more than KSh 8 a litre in a month, and come down more slowly.",
          "Fares on the busiest routes have risen twice this year. The association publishes its fare list at each stage.",
          "This explainer will be updated after each monthly review.",
      ], kicker="Explainer", ent="subscriber")
story("ward-offices-to-open-on-saturdays", "council", "Ward offices to open on Saturdays from December",
      "Residents who work weekdays will be able to get permits and letters on Saturday mornings.",
      "au-amani", "d-3 15:00", [
          "Every ward office in the county will open from 09:00 to 13:00 on Saturdays from December, the council's administration committee has decided.",
          "Residents will be able to apply for business permits and collect letters of introduction without taking a day off work.",
      ])
story("kivuli-primary-opens-its-new-library-block", "community", "Kivuli Primary opens its new library block",
      "Parents raised half the cost; the ward development fund paid the rest.",
      "au-lea", "d-2 06:00", [
          "Kivuli Primary School has opened a library block with room for 60 pupils and about 3,000 books.",
          "Parents raised half the cost through monthly contributions over two years. The ward development fund paid the rest.",
          "The head teacher says the library will open on Saturday mornings for pupils from other schools in the ward.",
      ], updated="d-2 09:40")
story("maize-buyers-offer-less-than-last-season", "farming", "Maize buyers offer less than last season",
      "Prices at the cooperative are down by about a fifth. Our figures from five buying points show who pays what.",
      "au-zawadi", "d-1 12:00", [
          "Maize buyers at the valley's five main buying points are offering about a fifth less per bag than at this time last year.",
          "The cooperative at Mto Mkuu pays the most, but only to members and only for dried grain. Roadside buyers pay cash but less.",
          "Farmers who can store their maize dry for two months have usually been paid more, according to the cooperative's records.",
          "The Times will print the buying points' prices every week until the end of the season.",
      ], ent="subscriber")
story("riverside-ward-clean-up-draws-200-volunteers", "community", "Riverside ward clean-up draws 200 volunteers",
      "Volunteers cleared the river bank from the bridge to the market.",
      "au-lea", "d-0 08:00", [
          "About 200 volunteers cleared rubbish from the river bank in Riverside ward, from the old bridge down to Ndovu market.",
          "The council lent two lorries and the market traders' association gave gloves and sacks. The next clean-up is planned for the end of the month.",
      ])
story("kora-junction-bridge-reopens-to-lorries", "transport", "Kora Junction bridge reopens to lorries",
      "The repaired bridge takes lorries up to 20 tonnes again after eight months.",
      "au-amani", "d-8 07:00", [
          "The Kora Junction bridge has reopened to lorries of up to 20 tonnes, eight months after cracks closed it to anything heavier than a pickup.",
          "Farmers at the cooperative say the closure added two hours to every trip to the cold store.",
      ])
story("extension-officers-warn-of-fall-armyworm", "farming", "Extension officers warn of fall armyworm after early rain",
      "Check maize leaves every three days, officers advise.",
      "au-zawadi", "d-10 08:00", [
          "The county's extension officers have warned that the early rain could bring fall armyworm into young maize.",
          "They advise farmers to check the leaves every three days and to report damage to the cooperative office, which keeps approved sprays.",
      ])
story("second-bank-branch-opens-in-kanyonga", "business", "Second bank branch opens in Kanyonga",
      "Traders welcome a shorter queue on market day.",
      "au-daniel", "d-11 11:00", [
          "A second bank branch has opened in Kanyonga, on the main road opposite the bus stage.",
          "Traders say market day queues at the old branch often lasted two hours.",
      ])
story("mto-mkuu-stars-reach-the-regional-cup-final", "community", "Mto Mkuu Stars reach the regional cup final",
      "A late goal sends the valley's football club to its first final in ten years.",
      "au-lea", "d-12 09:00", [
          "Mto Mkuu Stars beat their hosts 2 to 1 with a goal in the last minute to reach the regional cup final, the club's first in ten years.",
          "The final is at Kanyonga Stadium at the end of the month. The club is selling tickets at its office beside the bus park.",
      ], updated="d-11 18:00")
story("where-the-valley-milk-money-goes", "business", "Where the valley's milk money goes",
      "We followed a litre of milk from a Mlimani farm to a shop shelf in Mto Mkuu.",
      "au-daniel", "d-13 06:00", [
          "A dairy farmer in Mlimani is paid about KSh 45 for a litre of milk at the collection point. The same litre sells for about KSh 70 in a Mto Mkuu shop.",
          "The difference pays for cooling, transport to the processor in Kanyonga, packaging and the shop's margin.",
          "Farmers in the cooperative are paid KSh 5 a litre more than those who sell to private collectors, but wait a month for the money.",
          "The cooperative says a second cooler at Kora would let it collect from 300 more farms.",
      ], ent="subscriber")
story("matatu-operators-agree-new-stage-at-the-hospital", "transport", "Matatu operators agree a new stage at the hospital",
      "Patients will no longer need to walk from the main road.",
      "au-amani", "d-16 06:00", [
          "Matatu operators and the county have agreed a new stage at the gate of Mto Mkuu hospital.",
          "Until now, patients and visitors have had to walk about a kilometre from the main road stage.",
      ])
story("street-lights-for-market-road", "council", "Council adopts a plan for street lights on Market Road",
      "Forty solar lights will go up between the bus park and Ndovu market.",
      "au-amani", "d-19 06:00", [
          "The council has adopted a plan to put up 40 solar street lights on Market Road, between the bus park and Ndovu market.",
          "Traders asked for the lights after a run of evening robberies. Work starts once the contractor is chosen.",
      ])
story("short-rains-forecast-to-start-early", "farming", "Short rains forecast to start early",
      "The county weather office says farmers should prepare land now.",
      "au-zawadi", "d-23 06:00", [
          "The county weather office forecasts that the short rains will start earlier than usual this season.",
          "Extension officers advise farmers to prepare land and buy seed now, before prices rise.",
      ])
story("mlimani-health-centre-adds-a-night-shift", "community", "Health centre in Mlimani adds a night shift",
      "The centre now stays open all night for births and emergencies.",
      "au-lea", "d-30 06:00", [
          "Mlimani health centre now has a nurse and a clinical officer on duty every night, for births and emergencies.",
          "Until now, mothers in labour at night had to travel to Mto Mkuu hospital, about 25 km away.",
      ])
story("how-the-ward-bursary-money-was-shared", "council", "How the ward bursary money was shared",
      "Our count of the bursary lists from every ward shows big gaps between wards.",
      "au-amani", "d-33 06:00", [
          "The Times collected the bursary lists posted at every ward office this term. Together they show about 4,100 students received help with school fees.",
          "Riverside ward gave the most bursaries but the smallest amounts. Kivuli gave fewer, larger awards.",
          "The council's education committee says it will publish one county list next term, with the rules each ward used.",
      ], ent="subscriber")
story("road-to-mbalini-closed-for-culvert-works", "transport", "Road to Mbalini closed for culvert works",
      "Drivers should use the Kora road for three weeks.",
      "au-amani", "d-37 06:00", [
          "The road to Mbalini is closed at the second river crossing for three weeks while new culverts are laid.",
          "Drivers should use the Kora road. Matatus to Mbalini will run the longer route at the same fare, the operators say.",
      ])
story("cooperative-opens-a-second-cold-store", "farming", "Farmers' cooperative opens a second cold store",
      "The store at Kora holds 40 tonnes of fruit and vegetables.",
      "au-zawadi", "d-40 06:00", [
          "The valley farmers' cooperative has opened a second cold store at Kora, with room for 40 tonnes of fruit and vegetables.",
          "Members can store produce for up to ten days while they wait for a better price.",
      ])
story("hardware-prices-rise-ahead-of-building-season", "business", "Hardware prices rise ahead of the building season",
      "Cement and iron sheets cost more than a month ago in every shop we checked.",
      "au-daniel", "d-44 06:00", [
          "A bag of cement costs between KSh 50 and KSh 80 more than a month ago in the six hardware shops the Times checked in Mto Mkuu and Kanyonga.",
          "Shop owners blame transport costs from the coast. Iron sheets have risen by a similar amount.",
      ])
story("valley-schools-sit-final-term-exams", "community", "Valley schools sit their final term exams",
      "About 9,000 pupils across the county start exams this week.",
      "au-lea", "d-47 06:00", [
          "About 9,000 pupils in the county's primary and secondary schools start their final term exams this week.",
          "The county education office has asked matatu operators to give pupils in uniform priority in the mornings.",
      ])
story("avocado-growers-sign-a-new-buyer", "farming", "Avocado growers sign a new buyer",
      "The upper valley's growers will sell to an exporter in Kanyonga.",
      "au-zawadi", "d-55 06:00", [
          "Avocado growers in the upper valley have signed a two-year agreement to sell to an exporter with a packhouse in Kanyonga.",
          "The growers' group says the exporter will collect from the farm gate and pay within two weeks.",
      ])
story("new-bus-shelter-design-chosen", "transport", "New bus shelter design chosen by residents",
      "More than 1,200 people voted at ward offices.",
      "au-amani", "d-75 06:00", [
          "Residents have chosen the design for the county's new bus shelters, with a wider roof and a bench, in a vote at ward offices.",
          "More than 1,200 people voted. The first ten shelters go up on the Mto Mkuu to Kanyonga road.",
      ])
story("council-approves-a-new-water-tariff", "council", "Council approves a new water tariff",
      "Households will pay more for water above the first 6 cubic metres a month.",
      "au-amani", "d-110 06:00", [
          "The council has approved a new water tariff. Households keep the same price for their first 6 cubic metres a month and pay KSh 85 per cubic metre above that.",
          "The water company says the change pays for pipes to Mbalini and Kora, where families still buy water from carts.",
      ], updated="d-109 10:00")
story("mto-mkuu-library-starts-saturday-reading-club", "community", "Mto Mkuu library starts a Saturday reading club",
      "Children aged 6 to 12 can join for free.",
      "au-lea", "d-130 06:00", [
          "Mto Mkuu library has started a reading club for children aged 6 to 12 on Saturday mornings.",
          "Volunteers from the teachers' college run the sessions. Children can join for free with a parent's signature.",
      ])
story("what-it-costs-to-run-a-matatu", "business", "What it costs to run a matatu in the valley",
      "One owner opened her books to us for a month.",
      "au-daniel", "d-150 06:00", [
          "A matatu owner on the Kanyonga route let the Times follow her costs for a month. Fuel took about half of what the vehicle earned.",
          "Loan repayments, the driver's and conductor's pay, insurance and spares took most of the rest. In a good month she kept about KSh 30,000.",
          "She says a rise in diesel prices takes about a week of profit.",
      ], ent="subscriber")
story("long-rains-leave-dams-full", "farming", "Long rains leave dams full for the first time in four years",
      "The valley's farm dams are full, and the water company lifts rationing.",
      "au-zawadi", "d-170 06:00", [
          "The long rains have filled the valley's farm dams for the first time in four years, the county water office says.",
          "The water company has lifted the rationing timetable in Mto Mkuu and Kanyonga.",
      ])
story("how-the-county-budget-is-made", "council", "How the county budget is made, step by step",
      "From the ward meetings to the final vote, here is who decides what the county spends.",
      "au-amani", "d-200 06:00", [
          "The county's budget year starts in July. Work on it starts nine months earlier, with the finance department's estimate of how much money will come in.",
          "Ward meetings come next. Residents say what they want built or fixed, and ward councillors turn that into a list of requests.",
          "Departments then present their plans to the finance committee, which weighs them against the money expected and the wards' lists.",
          "The full council debates the committee's draft and votes on it. Once it passes, the county executive spends it, and the auditor checks the accounts the following year.",
          "Residents can speak at the ward meetings and attend the finance committee's hearings. Both are open to the public.",
      ], kicker="Explainer", topic="budget")
story("kanyonga-weekly-market-gets-a-roof", "business", "Kanyonga weekly market gets a roof",
      "Traders can now sell through the rain.",
      "au-daniel", "d-230 06:00", [
          "Kanyonga's weekly market now has a roof over its main hall, paid for by the ward development fund.",
          "Traders say rain used to empty the market by midday in the wet months.",
      ])
story("volunteer-firefighters-get-a-second-engine", "community", "Volunteer firefighters get a second engine",
      "The engine will be kept at Kanyonga, closer to the markets.",
      "au-lea", "d-260 06:00", [
          "The valley's volunteer fire brigade has a second fire engine, donated by a neighbouring county.",
          "It will be kept at Kanyonga so that fires at the markets can be reached in less time.",
      ])
story("coffee-prices-lift-growers-in-the-upper-valley", "farming", "Coffee prices lift growers in the upper valley",
      "The cooperative paid its members more per kilo than in any year since it opened.",
      "au-zawadi", "d-290 06:00", [
          "Coffee growers in the upper valley have been paid more per kilo of cherry this season than in any year since their cooperative opened.",
          "The cooperative says better prices at auction and fewer rejected bags explain the rise.",
      ])
story("footbridge-replaces-the-kora-ferry", "transport", "Footbridge replaces the Kora ferry",
      "Pupils who crossed the river by boat can now walk to school.",
      "au-amani", "d-320 06:00", [
          "A steel footbridge has replaced the boat that carried people across the river at Kora.",
          "About 400 pupils cross each school day. The county says the bridge can carry bicycles and motorbikes but not cars.",
      ])
story("plans-to-tarmac-the-kivuli-road-shelved", "transport", "Plans to tarmac the Kivuli road shelved",
      "The money goes to bridge repairs instead.",
      "au-amani", "d-380 06:00", [
          "The council has shelved its plan to tarmac the Kivuli road this year and moved the money to repairing the Kora Junction bridge.",
          "Kivuli residents say the road is impassable for a week after heavy rain.",
      ])
story("mto-mkuu-hospital-opens-maternity-wing", "community", "Mto Mkuu hospital opens its maternity wing",
      "The wing has 40 beds and its own theatre.",
      "au-lea", "d-450 06:00", [
          "Mto Mkuu hospital has opened a maternity wing with 40 beds and its own operating theatre.",
          "Until now, mothers who needed a caesarean were referred to Kanyonga.",
      ])
story("dry-spell-cuts-the-bean-harvest", "farming", "Dry spell cuts the bean harvest by a third",
      "Farmers in the lower valley were hit hardest.",
      "au-zawadi", "d-480 06:00", [
          "A six-week dry spell has cut the valley's bean harvest by about a third, according to the cooperative's records.",
          "Farmers in the lower valley, where there is no irrigation, were hit hardest.",
      ])
story("new-ward-boundaries-take-effect", "council", "New ward boundaries take effect",
      "Two wards are split and one new ward is created.",
      "au-amani", "d-540 06:00", [
          "New ward boundaries have taken effect in the county. Riverside and Mlimani are each split, and a new ward, Kora, is created.",
          "Residents can check their ward at the county hall or at any ward office.",
      ])
story("valley-marathon-returns", "community", "Valley marathon returns after four years",
      "More than 800 runners took part, from children to grandparents.",
      "au-lea", "d-580 06:00", [
          "The valley marathon has returned after four years, with more than 800 runners on the route from Kivuli to Mto Mkuu.",
          "There was a 5 km race for children and a 10 km race for anyone who did not want to run the full distance.",
      ])
story("why-shop-rents-in-mto-mkuu-doubled", "business", "Why shop rents in Mto Mkuu doubled",
      "We compared leases from five years ago with today's.",
      "au-daniel", "d-620 06:00", [
          "Shop rents on Mto Mkuu's main street have about doubled in five years, according to leases shown to the Times by twelve shop owners.",
          "Landlords point to the new tarmac and the bus park. Shop owners say rents have risen faster than what they sell.",
          "Two landlords told the Times they would hold rents for tenants who sign longer leases.",
      ], ent="subscriber")
story("irrigation-scheme-reaches-400-farms", "farming", "Irrigation scheme reaches 400 farms",
      "Farmers in the lower valley can now water crops through the dry months.",
      "au-zawadi", "d-660 06:00", [
          "The lower valley irrigation scheme now reaches 400 farms, after its third canal was finished.",
          "Farmers pay a yearly fee based on the size of their plots.",
      ])
story("council-moves-into-its-new-offices", "council", "Council moves into its new offices",
      "The county hall in Mto Mkuu replaces three rented buildings.",
      "au-amani", "d-700 06:00", [
          "The county council has moved into its new county hall in Mto Mkuu, replacing three rented buildings across town.",
          "The public gallery for council sittings has room for 120 people.",
      ])
story("kora-junction-bridge-plans-published", "transport", "Kora Junction bridge plans published",
      "Engineers say the old bridge needs strengthening within five years.",
      "au-amani", "d-740 06:00", [
          "The county has published an engineers' report on the Kora Junction bridge, which says it needs strengthening within five years.",
          "The report recommends limiting lorries to 20 tonnes until the work is done.",
      ])
story("kivuli-youth-centre-opens", "community", "Kivuli youth centre opens",
      "The centre has a computer room, a library corner and a football pitch.",
      "au-lea", "d-780 06:00", [
          "A youth centre has opened in Kivuli with a computer room, a library corner and a football pitch.",
          "It is open every afternoon and all day on Saturdays.",
      ])
story("mango-growers-form-a-cooperative", "farming", "Mango growers form a cooperative",
      "Growers hope to sell together and waste less fruit.",
      "au-zawadi", "d-830 06:00", [
          "Mango growers in the lower valley have formed a cooperative so they can sell together and agree prices with buyers.",
          "Members say up to half of their fruit has rotted in good years because buyers came late.",
      ])
story("mobile-money-agents-outnumber-banks", "business", "Mobile money agents outnumber banks in the valley",
      "Our count found more than 200 agents and four bank branches.",
      "au-daniel", "d-880 06:00", [
          "The Times counted more than 200 mobile money agents in the valley's towns and four bank branches.",
          "Most traders say they use agents for daily takings and banks only for loans.",
      ])
story("council-sets-up-a-ward-development-fund", "council", "Council sets up a ward development fund",
      "Each ward gets money to spend on projects its residents choose.",
      "au-amani", "d-930 06:00", [
          "The county council has set up a ward development fund, giving each ward money to spend on small projects its residents choose.",
          "Ward committees will publish what they spend on the ward office notice board.",
      ])
story("mlimani-clinic-gets-solar-power", "community", "Mlimani clinic gets solar power",
      "The clinic can now keep vaccines cold through power cuts.",
      "au-lea", "d-980 06:00", [
          "Mlimani clinic now runs on solar power, with batteries that keep its vaccine fridge cold through power cuts.",
          "Nurses say they used to send vaccines to Mto Mkuu when the power went off for more than a day.",
      ])
story("night-buses-between-mto-mkuu-and-kanyonga", "transport", "Night buses start between Mto Mkuu and Kanyonga",
      "Buses run every hour until midnight.",
      "au-amani", "d-1030 06:00", [
          "Buses now run every hour until midnight between Mto Mkuu and Kanyonga, after operators and the county agreed a night service.",
          "Hospital and market workers asked for the service, saying the last matatu left at 20:00.",
      ])

# ---------------------------------------------------------------- this week
story("kanyonga-market-gets-a-roof-before-the-short-rains", "community", "Kanyonga market gets a roof before the short rains",
      "Traders raised half the cost; the ward fund paid the rest, and the roof went up in nine days.",
      "au-lea", "d-1 18:00", [
          "Kanyonga's open-air market has a roof for the first time, after traders and the ward development fund split the KSh 2.4m cost.",
          "The traders' association collected weekly contributions for eight months. The fund approved the rest in October.",
          "Stall holders say the roof means they can stay open on wet afternoons. The market committee plans gutters and a water tank next.",
      ])
story("milk-cooperative-pays-farmers-weekly", "farming", "Milk cooperative starts paying farmers every week",
      "The upper valley cooperative moves from monthly to weekly payments after members voted for it in October.",
      "au-zawadi", "d-2 15:30", [
          "The Upper Valley Dairy Cooperative will pay its 1,300 members every Friday instead of at the end of each month, from this week.",
          "Members voted for the change at the October meeting. The cooperative says its new mobile payment account makes it possible.",
          "The price per litre stays at KSh 52 until the next review in January.",
      ])
story("council-names-a-date-for-the-budget-vote", "council", "Council names a date for the budget vote",
      "Members will vote on the county budget on Thursday 26 November, after the last ward hearings.",
      "au-amani", "d-1 11:20", [
          "The county assembly will vote on next year's budget on Thursday 26 November, the speaker said.",
          "The last ward hearings run this week in Mto Mkuu and Kivuli. Written comments close on Friday.",
      ])

write("stories", [{k: v for k, v in s.items() if v is not None} for s in S])

write("corrections", [
    {"story": "council-approves-first-phase-of-the-northern-rail-extension", "at": "d-3 16:20", "what": "KSh 41bn, not 48bn",
     "note": "An earlier version of this story gave the projected cost of phase one as KSh 48bn. The council's published estimate is KSh 41bn."},
    {"story": "short-rains-arrive-early-across-the-valley", "at": "d-5 12:10", "what": "112 mm, not 121 mm",
     "note": "An earlier version of this story said 121 mm of rain fell at the Mto Mkuu gauge. The gauge recorded 112 mm."},
    {"story": "ndovu-market-traders-get-a-date-for-the-move", "at": "d-2 11:30", "what": "February, not March",
     "note": "An earlier version of this story said the traders would move in March. The council has told them February."},
    {"story": "kivuli-primary-opens-its-new-library-block", "at": "d-2 09:40", "what": "3,000 books, not 300",
     "note": "An earlier version of this story said the library holds about 300 books. It holds about 3,000."},
    {"story": "mto-mkuu-stars-reach-the-regional-cup-final", "at": "d-11 18:00", "what": "Kanyonga Stadium, not Mto Mkuu Stadium",
     "note": "An earlier version of this story gave the venue of the final as Mto Mkuu Stadium. The final is at Kanyonga Stadium."},
    {"story": "council-approves-a-new-water-tariff", "at": "d-109 10:00", "what": "KSh 85, not KSh 58",
     "note": "An earlier version of this story gave the new price above 6 cubic metres as KSh 58. It is KSh 85 per cubic metre."},
])

write("timeline", [
    {"topic": "rail", "at": "d-420 06:00", "line": "A feasibility study backs a northern line", "story": "feasibility-study-backs-a-northern-rail-line"},
    {"topic": "rail", "at": "d-90 06:00", "line": "Public hearings on the route begin", "story": "public-hearings-on-the-northern-line-route-begin"},
    {"topic": "rail", "at": "d-6 09:00", "line": "Council publishes the extension proposal", "story": "council-publishes-the-northern-rail-extension-proposal"},
    {"topic": "rail", "at": "d-4 10:30", "line": "Residents split over the route", "story": "residents-split-over-the-northern-line-route"},
    {"topic": "rail", "at": "d-3 11:04", "line": "Phase one approved, 31 votes to 12", "story": "council-approves-first-phase-of-the-northern-rail-extension"},
    {"topic": "rail", "at": "d-3 16:20", "line": "Projected cost corrected to KSh 41bn (not 48bn)", "story": "council-approves-first-phase-of-the-northern-rail-extension", "kind": "correction"},
    {"topic": "rail", "at": "d+60 09:00", "line": "Construction tenders open", "kind": "scheduled"},
    {"topic": "budget", "at": "d-350 06:00", "line": "Last year's accounts: 87 percent of building money spent", "story": "last-year-budget-what-was-spent"},
    {"topic": "budget", "at": "d-200 06:00", "line": "Explainer: how the budget is made", "story": "how-the-county-budget-is-made"},
    {"topic": "budget", "at": "d-51 06:00", "line": "Ward meetings on next year's budget open", "story": "council-asks-residents-what-the-budget-should-fund"},
    {"topic": "budget", "at": "d-9 10:00", "line": "Departments defend their plans in public", "story": "county-budget-hearings-start-in-mto-mkuu"},
    {"topic": "budget", "at": "d-1 09:15", "line": "Wards asked to cut their requests", "story": "finance-committee-asks-wards-to-cut-spending-requests"},
    {"topic": "budget", "at": "d+24 10:00", "line": "The full council votes on the budget", "kind": "scheduled"},
    {"topic": "market", "at": "d-1080 06:00", "line": "Traders form an association", "story": "ndovu-market-traders-form-an-association"},
    {"topic": "market", "at": "d-510 06:00", "line": "Fire destroys about 60 stalls", "story": "ndovu-market-fire-traders-count-their-losses"},
    {"topic": "market", "at": "d-26 06:00", "line": "The council publishes its plan to move the market", "story": "ndovu-market-relocation-plan-published"},
    {"topic": "market", "at": "d-2 07:30", "line": "Traders told they move in February", "story": "ndovu-market-traders-get-a-date-for-the-move"},
    {"topic": "market", "at": "d-2 11:30", "line": "Moving month corrected to February (not March)", "story": "ndovu-market-traders-get-a-date-for-the-move", "kind": "correction"},
    {"topic": "market", "at": "d+85 08:00", "line": "Traders move to the new site", "kind": "scheduled"},
])

# Readers: two named seed readers (one subscriber, one not) and 38 sample readers.
sample_names = [
    "Wairimu N.", "Otieno K.", "Chebet R.", "Kamau J.", "Nyambura W.", "Odhiambo P.", "Wambua M.", "Jeptoo L.",
    "Mwende S.", "Kiprono T.", "Nduta G.", "Omondi B.", "Wanjala E.", "Akinyi F.", "Mutua D.", "Kerubo H.",
    "Gitau A.", "Moraa C.", "Kiplagat N.", "Wangari V.", "Barasa I.", "Nekesa O.", "Karanja Y.", "Auma Z.",
    "Kimani Q.", "Jelagat U.", "Ouma X.", "Wanjiku T.", "Rotich S.", "Nafula R.", "Njoroge M.", "Muthoni E.",
    "Kibet A.", "Atieno L.", "Mwangi C.", "Chepkoech D.", "Okoth F.", "Wekesa G.",
]
customers = [
    {"customer_id": "cus-halima", "name": "Halima A. (Sample reader)", "email": "halima@example.com", "place": "Mto Mkuu", "created_at": "d-400 19:02"},
    {"customer_id": "cus-baraka", "name": "Baraka M. (Sample reader)", "email": "baraka@example.com", "place": "Kanyonga", "created_at": "d-20 08:10"},
]
for i, name in enumerate(sample_names, start=1):
    customers.append({"customer_id": f"cus-r{i:02d}", "name": f"{name} (Sample reader)", "email": f"reader{i:02d}@example.com",
                      "place": ["Mto Mkuu", "Kanyonga", "Kivuli", "Kora", "Mlimani"][i % 5], "created_at": f"d-{30 + i * 17} 09:00"})
write("customers", customers)

subscribers = [{"subscriber_id": "sub-halima", "customer": "cus-halima", "plan": "Supporter", "source": "owner grant", "since": "d-380 10:00"}]
for i in range(1, 12):
    row = {"subscriber_id": f"sub-r{i:02d}", "customer": f"cus-r{i:02d}", "plan": "Supporter", "since": f"d-{20 + i * 23} 10:00"}
    if i % 4 == 0:
        # One sandbox payment buys one month: it started 30 days before it ends.
        row.update({"source": "sandbox payment", "since": f"d-{30 - i * 2} 10:00", "until": f"d+{i * 2} 10:00"})
    else:
        row["source"] = "owner grant"
    subscribers.append(row)
write("subscribers", subscribers)

TOPIC_SETS = [["Transport"], ["Council", "Transport"], ["Farming and weather"], ["Business"], ["Community"], ["Transport", "Business"],
              ["Farming and weather", "Business"], []]
digest = [{"sub_id": "dg-halima", "email": "halima@example.com", "topics": ["Transport", "Council"], "state": "confirmed",
           "created_at": "d-300 08:00", "confirmed_at": "d-300 08:04"}]
for i in range(1, 25):
    digest.append({"sub_id": f"dg-r{i:02d}", "email": f"reader{i + 10:02d}@example.com", "topics": TOPIC_SETS[i % len(TOPIC_SETS)],
                   "state": "confirmed", "created_at": f"d-{10 + i * 9} 07:30", "confirmed_at": f"d-{10 + i * 9} 07:41"})
# Two readers who signed up this week and haven't confirmed yet: nothing is sent until they do.
digest += [
    {"sub_id": "dg-p01", "email": "reader38@example.com", "topics": ["Community"], "state": "pending", "created_at": "d-1 20:15"},
    {"sub_id": "dg-p02", "email": "reader39@example.com", "topics": ["Farming and weather"], "state": "pending", "created_at": "m-140"},
]
write("digest_subs", digest)

write("questions", [
    {"question_id": "q-t1", "desk": "Newsroom", "question": "Will the rail extension stop at Mbalini in phase two?", "contact": "reader07@example.com", "page": "/transport", "at": "d-2 19:40"},
    {"question_id": "q-t2", "desk": "Subscriptions", "question": "Can our school get a subscription for the teachers' room?", "contact": "reader21@example.com", "page": "/subscribe", "at": "m-300"},
])
print("wrote", os.path.normpath(OUT), len(S), "stories")
