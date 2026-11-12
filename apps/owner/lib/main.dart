// Agentrix OS on the phone (concept/console/phone.html; 13 §7.2): pair once with the console,
// then Now, Decisions, Live and Apps. Approving needs an authenticator code bound to the card,
// or opens the card in the console for a passkey. Swiping never approves.
import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'src/apps.dart';
import 'src/decisions.dart';
import 'src/live.dart';
import 'src/now.dart';
import 'src/pair.dart';

/// The platform's origin. Local runs pass `--dart-define=AGENTRIX_ORIGIN=http://localhost:18050`.
const defaultOrigin = String.fromEnvironment('AGENTRIX_ORIGIN', defaultValue: 'https://agentrix.website');

void main() => runApp(const OwnerApp());

/// The app: pairing until a token is kept in the Keychain, then the phone's four tabs.
class OwnerApp extends StatefulWidget {
  const OwnerApp({super.key});

  @override
  State<OwnerApp> createState() => _OwnerAppState();
}

class _OwnerAppState extends State<OwnerApp> {
  static const _store = FlutterSecureStorage();
  OwnerApi? _api;
  bool _loading = true;
  bool _night = false;

  @override
  void initState() {
    super.initState();
    _restore();
  }

  Future<void> _restore() async {
    String? token;
    String? origin;
    try {
      token = await _store.read(key: 'token');
      origin = await _store.read(key: 'origin');
    } catch (_) {}
    setState(() {
      _api = token == null ? null : OwnerApi(origin: origin ?? defaultOrigin, token: token);
      _loading = false;
    });
  }

  Future<void> _paired(OwnerApi api) async {
    try {
      await _store.write(key: 'token', value: api.token);
      await _store.write(key: 'origin', value: api.origin);
    } catch (_) {}
    setState(() => _api = api);
  }

  Future<void> _unpaired() async {
    try {
      await _store.deleteAll();
    } catch (_) {}
    setState(() => _api = null);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Agentrix',
      debugShowCheckedModeBanner: false,
      theme: agxTheme(night: _night),
      home: _loading
          ? const Scaffold(body: Center(child: CircularProgressIndicator()))
          : _api == null
              ? PairScreen(defaultOrigin: defaultOrigin, onPaired: _paired)
              : Shell(api: _api!, onUnpaired: _unpaired, onNight: (n) {
                  if (n != _night) setState(() => _night = n);
                }),
    );
  }
}

/// The four tabs (`.ptabs`).
class Shell extends StatefulWidget {
  const Shell({super.key, required this.api, required this.onUnpaired, required this.onNight});

  final OwnerApi api;
  final VoidCallback onUnpaired;
  final ValueChanged<bool> onNight;

  @override
  State<Shell> createState() => _ShellState();
}

class _ShellState extends State<Shell> {
  int _tab = 0;
  int _waiting = 0;
  String? _openCard;

  void _open(String card) => setState(() {
        _openCard = card;
        _tab = 1;
      });

  @override
  Widget build(BuildContext context) {
    final pages = [
      NowScreen(
        api: widget.api,
        onOpen: _open,
        onUnpaired: widget.onUnpaired,
        onLoaded: (now) {
          final w = (now['waiting'] as num?)?.toInt() ?? 0;
          if (w != _waiting) setState(() => _waiting = w);
          widget.onNight(now['night'] == true);
        },
      ),
      DecisionsScreen(api: widget.api, open: _openCard, onUnpaired: widget.onUnpaired, onChanged: (n) {
        if (n != _waiting) setState(() => _waiting = n);
      }),
      LiveScreen(api: widget.api),
      AppsScreen(api: widget.api, onOpen: _open, onUnpaired: widget.onUnpaired),
    ];
    return Scaffold(
      body: SafeArea(bottom: false, child: IndexedStack(index: _tab, children: pages)),
      bottomNavigationBar: PhoneTabs(
        index: _tab,
        onTap: (i) => setState(() {
          _tab = i;
          if (i != 1) _openCard = null;
        }),
        tabs: [
          const PhoneTab('Now', Icons.wb_sunny_outlined),
          PhoneTab('Decisions', Icons.inbox_outlined, badge: _waiting),
          const PhoneTab('Live', Icons.sensors),
          const PhoneTab('Apps', Icons.grid_view_rounded),
        ],
      ),
    );
  }
}
