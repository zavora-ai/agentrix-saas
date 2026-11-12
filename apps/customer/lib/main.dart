// A sample site's own app (13 §7.1): one codebase, a flavour per site chosen at build time with
// `--dart-define=AGENTRIX_SITE=kilele|classy|tana` (and `AGENTRIX_SITE_ORIGIN` for a local node).
// Ask is the site's own Ask, so answers, desks, citations and refusals match the page; sign-in is
// W6's customer accounts on the site's host. Every screen says "Sample" where a person could take
// it for a real business.
import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:url_launcher/url_launcher.dart';

import 'src/flavour.dart';

void main() => runApp(CustomerApp(flavour: Flavour.current()));

/// The app for one site.
class CustomerApp extends StatelessWidget {
  const CustomerApp({super.key, required this.flavour});

  final Flavour flavour;

  @override
  Widget build(BuildContext context) => MaterialApp(
        title: flavour.brand.name,
        debugShowCheckedModeBanner: false,
        theme: agxTheme(brand: flavour.brand),
        home: Home(flavour: flavour, api: SiteApi(origin: flavour.origin)),
      );
}

/// Home, Ask and Account.
class Home extends StatefulWidget {
  const Home({super.key, required this.flavour, required this.api});

  final Flavour flavour;
  final SiteApi api;

  @override
  State<Home> createState() => _HomeState();
}

class _HomeState extends State<Home> {
  int _tab = 0;
  final _ask = GlobalKey<AskScreenState>();

  void _askAbout(String q) {
    setState(() => _tab = 1);
    WidgetsBinding.instance.addPostFrameCallback((_) => _ask.currentState?.send(q));
  }

  @override
  Widget build(BuildContext context) {
    final b = widget.flavour.brand;
    final pages = [
      FrontScreen(flavour: widget.flavour, onAsk: _askAbout),
      AskScreen(key: _ask, flavour: widget.flavour, api: widget.api),
      AccountScreen(flavour: widget.flavour, api: widget.api),
    ];
    return Scaffold(
      appBar: AppBar(
        backgroundColor: b.bg,
        title: Row(children: [
          Flexible(child: Text(b.name, overflow: TextOverflow.ellipsis, style: TextStyle(fontFamily: b.display, package: kitPackage, fontSize: 22, fontWeight: FontWeight.w600, color: b.ink))),
          const SizedBox(width: 8),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
            decoration: BoxDecoration(color: b.soft, borderRadius: BorderRadius.circular(999), border: Border.all(color: b.line)),
            child: Text('Sample', style: textStyle(11, weight: FontWeight.w700, color: b.ink2)),
          ),
        ]),
      ),
      body: IndexedStack(index: _tab, children: pages),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _tab,
        backgroundColor: b.bg,
        indicatorColor: b.soft,
        onDestinationSelected: (i) => setState(() => _tab = i),
        destinations: const [
          NavigationDestination(icon: Icon(Icons.storefront_outlined), label: 'Home'),
          NavigationDestination(icon: Icon(Icons.chat_bubble_outline), label: 'Ask'),
          NavigationDestination(icon: Icon(Icons.person_outline), label: 'Account'),
        ],
      ),
    );
  }
}

/// The site's front: what it is, its desks, and the questions people ask.
class FrontScreen extends StatelessWidget {
  const FrontScreen({super.key, required this.flavour, required this.onAsk});

  final Flavour flavour;
  final ValueChanged<String> onAsk;

  @override
  Widget build(BuildContext context) {
    final b = flavour.brand;
    return ListView(padding: const EdgeInsets.all(16), children: [
      Container(
        padding: const EdgeInsets.all(18),
        decoration: BoxDecoration(color: b.soft, borderRadius: BorderRadius.circular(20), border: Border.all(color: b.line)),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(flavour.headline, style: TextStyle(fontFamily: b.display, package: kitPackage, fontSize: 30, fontWeight: FontWeight.w600, color: b.ink, height: 1.1)),
          const SizedBox(height: 10),
          Text(flavour.intro, style: textStyle(15, color: b.ink2)),
          const SizedBox(height: 14),
          AgxButton('Open the site', icon: Icons.open_in_new, onPressed: () => launchUrl(Uri.parse(flavour.origin), mode: LaunchMode.externalApplication)),
        ]),
      ),
      const SizedBox(height: 18),
      Text('Ask the site', style: textStyle(13, weight: FontWeight.w700, color: b.ink)),
      const SizedBox(height: 4),
      Text("The site's WebAgents answer from its own pages and say so when the site doesn't say.", style: textStyle(13, color: b.ink2)),
      const SizedBox(height: 10),
      for (final q in flavour.starters)
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: AgxCard(
            flat: true,
            onTap: () => onAsk(q),
            child: Row(children: [
              Icon(Icons.chat_bubble_outline, size: 18, color: b.accent),
              const SizedBox(width: 10),
              Expanded(child: Text(q, style: textStyle(14, weight: FontWeight.w600, color: b.ink))),
              Icon(Icons.chevron_right, color: b.ink2),
            ]),
          ),
        ),
      const SizedBox(height: 10),
      Text("Sample site. ${b.name} is fictional. Payments run in PayPal's sandbox; no real money moves.", style: textStyle(12, color: b.ink2)),
    ]);
  }
}

/// One turn in the conversation.
class Turn {
  Turn.mine(this.text) : answer = null;
  Turn.answer(this.answer) : text = null;
  final String? text;
  final Json? answer;
}

/// Ask: the site's own Ask, desk by desk, with citations that open the page.
class AskScreen extends StatefulWidget {
  const AskScreen({super.key, required this.flavour, required this.api});

  final Flavour flavour;
  final SiteApi api;

  @override
  State<AskScreen> createState() => AskScreenState();
}

/// The Ask screen's state (Home sends starter questions here).
class AskScreenState extends State<AskScreen> {
  final _turns = <Turn>[];
  final _q = TextEditingController();
  bool _busy = false;
  String? _progress;

  /// Asks `q`.
  Future<void> send(String q) async {
    q = q.trim();
    if (q.isEmpty || _busy) return;
    setState(() {
      _turns.add(Turn.mine(q));
      _busy = true;
      _progress = 'Searching this site';
      _q.clear();
    });
    try {
      final events = await widget.api.ask(q);
      final answer = events.where((e) => e.kind == 'answer').map((e) => e.data).firstOrNull;
      setState(() => _turns.add(Turn.answer(answer ?? {'phrases': ['Answers are paused right now. The site still works.']})));
    } on ApiError catch (e) {
      setState(() => _turns.add(Turn.answer({'phrases': [e.message]})));
    } finally {
      setState(() {
        _busy = false;
        _progress = null;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final b = widget.flavour.brand;
    return Column(children: [
      Expanded(
        child: _turns.isEmpty
            ? Center(
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Text('Ask about ${widget.flavour.topics}. Each answer comes from the site and shows where it says so.', textAlign: TextAlign.center, style: textStyle(14, color: b.ink2)),
                ),
              )
            : ListView(padding: const EdgeInsets.all(14), children: [for (final t in _turns) _bubble(context, t)]),
      ),
      if (_progress != null) Padding(padding: const EdgeInsets.only(bottom: 6), child: Text('$_progress…', style: textStyle(12, color: b.ink2))),
      SafeArea(
        top: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 6, 12, 10),
          child: Row(children: [
            Expanded(child: TextField(controller: _q, textInputAction: TextInputAction.send, onSubmitted: send, decoration: InputDecoration(hintText: 'Ask ${b.name}'))),
            const SizedBox(width: 8),
            AgxButton('Ask', kind: AgxButtonKind.primary, onPressed: _busy ? null : () => send(_q.text)),
          ]),
        ),
      ),
    ]);
  }

  Widget _bubble(BuildContext context, Turn t) {
    final b = widget.flavour.brand;
    if (t.text != null) {
      return Align(
        alignment: Alignment.centerRight,
        child: Container(
          margin: const EdgeInsets.only(bottom: 10, left: 48),
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
          decoration: BoxDecoration(color: b.accent, borderRadius: BorderRadius.circular(16)),
          child: Text(t.text!, style: textStyle(14, color: b.onAccent)),
        ),
      );
    }
    final a = t.answer!;
    final desk = (a['desk'] as Json?)?['name'] as String? ?? 'Front desk';
    final phrases = (a['phrases'] as List? ?? const []).cast<String>().join(' ');
    final sources = (a['sources'] as List? ?? const []).cast<Json>();
    final actions = (a['actions'] as List? ?? const []).cast<Json>();
    return Container(
      margin: const EdgeInsets.only(bottom: 12, right: 24),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(color: b.bg, borderRadius: BorderRadius.circular(16), border: Border.all(color: b.line)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('$desk · WebAgent', style: textStyle(12, weight: FontWeight.w700, color: b.accent)),
        if (a['handover'] is String) Text(a['handover'] as String, style: textStyle(12, color: b.ink2).copyWith(fontStyle: FontStyle.italic)),
        const SizedBox(height: 6),
        Text.rich(TextSpan(children: _bold(phrases, textStyle(14, color: b.ink)))),
        for (final s in sources)
          InkWell(
            onTap: () => launchUrl(Uri.parse('${widget.flavour.origin}${s['href']}'), mode: LaunchMode.externalApplication),
            child: Padding(padding: const EdgeInsets.only(top: 6), child: Text('[${s['n']}] ${s['label']}', style: textStyle(13, weight: FontWeight.w600, color: b.accent2))),
          ),
        if ((a['meta'] as String? ?? '').isNotEmpty) Padding(padding: const EdgeInsets.only(top: 8), child: Text(a['meta'] as String, style: textStyle(12, color: b.ink2))),
        if (actions.isNotEmpty) ...[
          const SizedBox(height: 8),
          Wrap(spacing: 8, runSpacing: 8, children: [
            for (final act in actions.where((x) => x['kind'] == 'link'))
              AgxButton(act['label'] as String, small: true, kind: act['primary'] == true ? AgxButtonKind.primary : AgxButtonKind.plain, onPressed: () {
                final href = act['href'] as String;
                launchUrl(Uri.parse(href.startsWith('http') ? href : '${widget.flavour.origin}$href'), mode: LaunchMode.externalApplication);
              }),
          ]),
        ],
      ]),
    );
  }

  /// `**bold**` spans, as Ask writes them.
  List<TextSpan> _bold(String text, TextStyle base) {
    final out = <TextSpan>[];
    final parts = text.split('**');
    for (var i = 0; i < parts.length; i++) {
      out.add(TextSpan(text: parts[i], style: i.isOdd ? base.copyWith(fontWeight: FontWeight.w700) : base));
    }
    return out;
  }
}

/// Account: sign in with an email code (W6's customer accounts on the site's host).
class AccountScreen extends StatefulWidget {
  const AccountScreen({super.key, required this.flavour, required this.api});

  final Flavour flavour;
  final SiteApi api;

  @override
  State<AccountScreen> createState() => _AccountScreenState();
}

class _AccountScreenState extends State<AccountScreen> {
  static const _store = FlutterSecureStorage();
  final _email = TextEditingController();
  final _code = TextEditingController();
  String? _pending;
  String? _sentTo;
  Json? _session;
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _restore();
  }

  Future<void> _restore() async {
    try {
      widget.api.session = await _store.read(key: 'session-${widget.flavour.id}');
    } catch (_) {}
    final s = await widget.api.currentSession();
    if (mounted) setState(() => _session = s);
  }

  Future<void> _start() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final r = await widget.api.startCode(_email.text.trim());
      setState(() {
        _pending = r['pending'] as String?;
        _sentTo = r['sent_to'] as String?;
      });
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    } finally {
      setState(() => _busy = false);
    }
  }

  Future<void> _verify() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.api.verifyCode(_pending!, _code.text.trim());
      try {
        if (widget.api.session != null) await _store.write(key: 'session-${widget.flavour.id}', value: widget.api.session);
      } catch (_) {}
      final s = await widget.api.currentSession();
      setState(() {
        _session = s;
        _pending = null;
      });
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    } finally {
      setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final b = widget.flavour.brand;
    final signedIn = _session?['signed_in'] == true;
    final account = _session?['account'] as Json?;
    return ListView(padding: const EdgeInsets.all(16), children: [
      Text(signedIn ? 'Your account' : 'Sign in', style: TextStyle(fontFamily: b.display, package: kitPackage, fontSize: 26, fontWeight: FontWeight.w600, color: b.ink)),
      const SizedBox(height: 8),
      if (signedIn) ...[
        Text('Signed in as ${account?['name'] ?? account?['email'] ?? 'you'}.', style: textStyle(15, color: b.ink2)),
        const SizedBox(height: 12),
        AgxButton('See your orders on the site', onPressed: () => launchUrl(Uri.parse('${widget.flavour.origin}/orders'), mode: LaunchMode.externalApplication)),
        const SizedBox(height: 8),
        AgxButton('See your choices and what the site keeps', onPressed: () => launchUrl(Uri.parse('${widget.flavour.origin}/account/choices'), mode: LaunchMode.externalApplication)),
      ] else if (_pending == null) ...[
        Text('We email you a 6-digit code. No password.', style: textStyle(15, color: b.ink2)),
        const SizedBox(height: 12),
        TextField(controller: _email, keyboardType: TextInputType.emailAddress, autocorrect: false, decoration: const InputDecoration(labelText: 'Email')),
        const SizedBox(height: 12),
        if (_error != null) ...[Notice(_error!, tone: Tone.risk), const SizedBox(height: 10)],
        AgxButton(_busy ? 'Sending…' : 'Email me a code', kind: AgxButtonKind.primary, expand: true, onPressed: _busy ? null : _start),
      ] else ...[
        Text('We sent a code to ${_sentTo ?? 'your email'}. It works for 10 minutes.', style: textStyle(15, color: b.ink2)),
        const SizedBox(height: 12),
        TextField(controller: _code, keyboardType: TextInputType.number, style: monoStyle(22, color: b.ink), decoration: const InputDecoration(labelText: 'Code')),
        const SizedBox(height: 12),
        if (_error != null) ...[Notice(_error!, tone: Tone.risk), const SizedBox(height: 10)],
        AgxButton(_busy ? 'Checking…' : 'Sign in', kind: AgxButtonKind.primary, expand: true, onPressed: _busy ? null : _verify),
      ],
      const SizedBox(height: 18),
      Text("Passkeys don't work in the app yet. Sign in here with a code, or on ${widget.flavour.host} with your passkey.", style: textStyle(12, color: b.ink2)),
    ]);
  }
}
