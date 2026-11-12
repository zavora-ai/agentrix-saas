import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';

/// Pair this phone with the console: the owner types the code from Settings → Channels.
class PairScreen extends StatefulWidget {
  const PairScreen({super.key, required this.defaultOrigin, required this.onPaired});

  final String defaultOrigin;
  final ValueChanged<OwnerApi> onPaired;

  @override
  State<PairScreen> createState() => _PairScreenState();
}

class _PairScreenState extends State<PairScreen> {
  final _code = TextEditingController();
  late final _origin = TextEditingController(text: widget.defaultOrigin);
  final _device = TextEditingController(text: 'My phone');
  bool _busy = false;
  String? _error;
  bool _showOrigin = false;

  Future<void> _pair() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    final api = OwnerApi(origin: _origin.text.trim());
    try {
      await api.pair(_code.text, _device.text);
      widget.onPaired(api);
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    return Scaffold(
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(20, 32, 20, 24),
          children: [
            const Align(alignment: Alignment.centerLeft, child: TeamMark(size: 56, letter: 'A')),
            const SizedBox(height: 20),
            Text('Pair this phone', style: displayStyle(26, color: c.indigo900)),
            const SizedBox(height: 8),
            Text(
              'On your computer, open the console, then Settings → Channels, and click Pair a phone. Type the code it shows. It works once, for 10 minutes.',
              style: textStyle(15, color: c.ink2),
            ),
            const SizedBox(height: 20),
            TextField(
              controller: _code,
              textCapitalization: TextCapitalization.characters,
              autocorrect: false,
              style: monoStyle(22, color: c.indigo900),
              decoration: const InputDecoration(labelText: 'Pairing code', hintText: 'ABCD-EFGH'),
              onSubmitted: (_) => _pair(),
            ),
            const SizedBox(height: 12),
            TextField(controller: _device, decoration: const InputDecoration(labelText: "This phone's name, as the console lists it")),
            const SizedBox(height: 16),
            if (_error != null) ...[Notice(_error!, tone: Tone.risk), const SizedBox(height: 12)],
            AgxButton(_busy ? 'Pairing…' : 'Pair', kind: AgxButtonKind.primary, large: true, expand: true, onPressed: _busy ? null : _pair),
            const SizedBox(height: 20),
            Text(
              'Pairing lets this phone show your brief, what needs you and your WebAgents. It never approves on its own: every approval needs your authenticator code or your passkey.',
              style: textStyle(13, color: c.muted),
            ),
            const SizedBox(height: 12),
            if (!_showOrigin)
              Align(
                alignment: Alignment.centerLeft,
                child: AgxButton('Use another address', kind: AgxButtonKind.ghost, small: true, onPressed: () => setState(() => _showOrigin = true)),
              )
            else
              TextField(controller: _origin, keyboardType: TextInputType.url, autocorrect: false, decoration: const InputDecoration(labelText: 'Console address')),
          ],
        ),
      ),
    );
  }
}
