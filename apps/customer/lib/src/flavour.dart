import 'package:agentrix_kit/agentrix_kit.dart';

/// One site's flavour of the customer app. The words come from each sample's own bundle
/// (`fixtures/samples/<site>/manifest.json`) and are marked Sample on screen.
class Flavour {
  const Flavour({required this.id, required this.brand, required this.host, required this.origin, required this.headline, required this.intro, required this.topics, required this.starters});

  /// `kilele`, `classy` or `tana`.
  final String id;

  /// Colours and display face, generated from the site's theme.
  final SiteBrand brand;

  /// The site's host.
  final String host;

  /// Where the app reaches the site.
  final String origin;

  /// The front's first line.
  final String headline;

  /// One paragraph about the site.
  final String intro;

  /// What people ask about, for the empty Ask screen.
  final String topics;

  /// Questions that show the desks at work.
  final List<String> starters;

  /// The flavour this build was made for (`--dart-define=AGENTRIX_SITE=…`).
  static Flavour current() {
    const site = String.fromEnvironment('AGENTRIX_SITE', defaultValue: 'kilele');
    const origin = String.fromEnvironment('AGENTRIX_SITE_ORIGIN');
    final f = all[site] ?? all['kilele']!;
    return origin.isEmpty ? f : f.at(origin);
  }

  /// The same flavour, reached at `origin` (a local node or a dev proxy).
  Flavour at(String origin) => Flavour(id: id, brand: brand, host: host, origin: origin, headline: headline, intro: intro, topics: topics, starters: starters);

  /// Every flavour.
  static const all = {
    'kilele': Flavour(
      id: 'kilele',
      brand: kilele,
      host: 'kilele-bakery-sample.agentrix.website',
      origin: 'https://kilele-bakery-sample.agentrix.website',
      headline: 'Baked at dawn in Westlands.',
      intro: 'Sourdough, croissants and celebration cakes since 2016. Open from 6:30 Monday to Saturday and 7:00 on Sunday. Weekend delivery to Westlands and Kilimani.',
      topics: 'cakes, bread, opening hours, delivery or your order',
      starters: ['Do you have cakes for 10 people?', 'What time do you open on Sunday?', 'Do you deliver to Kilimani?'],
    ),
    'classy': Flavour(
      id: 'classy',
      brand: classy,
      host: 'classy-perfumes-sample.agentrix.website',
      origin: 'https://classy-perfumes-sample.agentrix.website',
      headline: 'Perfume and gift sets, wrapped free in Westlands.',
      intro: 'Perfumes in three sizes across five scent families, gift sets wrapped by hand, same-day delivery in Nairobi and courier delivery to 18 towns. Every name and price here is a sample.',
      topics: 'scents, sizes, gifts, delivery or your order',
      starters: ['What smells like rain?', 'Which is lightest?', 'Is Citrus Dawn in stock?'],
    ),
    'tana': Flavour(
      id: 'tana',
      brand: tana,
      host: 'tana-valley-times-sample.agentrix.website',
      origin: 'https://tana-valley-times-sample.agentrix.website',
      headline: "The valley's weekly paper, online every day.",
      intro: "A sample local paper. Its desks answer from Tana Valley Times' published stories and link every one, with its byline and date. They never write news.",
      topics: 'stories, the archive, the Friday five or your subscription',
      starters: ['Where does the rail extension stand?', 'Who has been reporting it?', 'Is the extension a good idea?'],
    ),
  };
}
