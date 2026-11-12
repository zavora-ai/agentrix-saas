import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';

/// Decisions: one card at a time (phone.html, third phone). The exact effect, why this risk,
/// Approve with an authenticator code bound to the card, or open it in the console for a passkey;
/// Decline.
class DecisionsScreen extends StatefulWidget {
  const DecisionsScreen({super.key, required this.api, this.open, required this.onUnpaired, required this.onChanged});

  final OwnerApi api;
  final String? open;
  final VoidCallback onUnpaired;
  final ValueChanged<int> onChanged;

  @override
  State<DecisionsScreen> createState() => _DecisionsScreenState();
}

class _DecisionsScreenState extends State<DecisionsScreen> {
  List<Json>? _cards;
  String? _error;
  int _index = 0;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(DecisionsScreen old) {
    super.didUpdateWidget(old);
    if (widget.open != old.open) _load();
  }

  Future<void> _load() async {
    try {
      final cards = await widget.api.cards();
      final at = widget.open == null ? 0 : cards.indexWhere((c) => c['id'] == widget.open);
      setState(() {
        _cards = cards;
        _index = at < 0 ? 0 : at;
        _error = null;
      });
      widget.onChanged(cards.length);
    } on ApiError catch (e) {
      if (e.unpaired) return widget.onUnpaired();
      setState(() => _error = e.message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final cards = _cards;
    if (cards == null) {
      return Center(child: _error == null ? const CircularProgressIndicator() : Padding(padding: const EdgeInsets.all(20), child: Notice(_error!, tone: Tone.risk)));
    }
    if (cards.isEmpty) {
      return RefreshIndicator(
        onRefresh: _load,
        child: ListView(padding: const EdgeInsets.all(20), children: [
          Text('Decisions', style: displayStyle(20, color: c.indigo900)),
          const SizedBox(height: 12),
          Text('Nothing is waiting for you to decide. When a WebAgent asks, the card appears here with exactly what approving would do.', style: textStyle(14, color: c.ink2)),
        ]),
      );
    }
    final i = _index.clamp(0, cards.length - 1);
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 24),
        children: [
          Row(children: [
            IconButton(
              tooltip: 'Previous decision',
              onPressed: i > 0 ? () => setState(() => _index = i - 1) : null,
              icon: const Icon(Icons.chevron_left),
            ),
            Text('Decisions · ${i + 1} of ${cards.length}', style: textStyle(13, color: c.muted)),
            IconButton(
              tooltip: 'Next decision',
              onPressed: i < cards.length - 1 ? () => setState(() => _index = i + 1) : null,
              icon: const Icon(Icons.chevron_right),
            ),
          ]),
          CardView(key: ValueKey(cards[i]['id']), api: widget.api, id: cards[i]['id'] as String, onDecided: _load),
        ],
      ),
    );
  }
}

/// One card in full.
class CardView extends StatefulWidget {
  const CardView({super.key, required this.api, required this.id, required this.onDecided});

  final OwnerApi api;
  final String id;
  final Future<void> Function() onDecided;

  @override
  State<CardView> createState() => _CardViewState();
}

class _CardViewState extends State<CardView> {
  Json? _card;
  String? _done;
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    widget.api.card(widget.id).then((c) => setState(() => _card = c)).catchError((Object e) {
      setState(() => _error = e.toString());
    });
  }

  Future<void> _decline() async {
    setState(() => _busy = true);
    try {
      final r = await widget.api.decline(widget.id);
      setState(() => _done = r['message'] as String? ?? 'Declined. Nothing changed.');
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    } finally {
      setState(() => _busy = false);
    }
  }

  Future<void> _openInConsole() async {
    final url = Uri.parse(_card!['url'] as String);
    if (!await launchUrl(url, mode: LaunchMode.externalApplication)) {
      setState(() => _error = 'Open ${url.toString()} in your browser to approve with your passkey.');
    }
  }

  Future<void> _approve() async {
    final result = await showModalBottomSheet<String>(
      context: context,
      isScrollControlled: true,
      backgroundColor: const Color(0xFF1F1D3D),
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(26))),
      builder: (_) => ApproveSheet(api: widget.api, card: _card!),
    );
    if (result != null) setState(() => _done = result);
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final card = _card;
    if (card == null) {
      return Padding(padding: const EdgeInsets.all(20), child: _error == null ? const Center(child: CircularProgressIndicator()) : Notice(_error!, tone: Tone.risk));
    }
    final agent = card['agent'] as String;
    final reasons = (card['reasons'] as List).cast<String>();
    final can = card['can'] as Json;
    final level = card['risk'] as String;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Align(alignment: Alignment.centerRight, child: AgxChip(card['risk_words'] as String, tone: toneForRisk(level))),
      const SizedBox(height: 8),
      AgxCard(
        flat: true,
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Row(children: [
            AgentAvatar(name: agent, size: 26),
            const SizedBox(width: 8),
            Text('${agent.replaceFirst('The ', '')} asks', style: textStyle(12, weight: FontWeight.w700, color: c.indigo900)),
          ]),
          const SizedBox(height: 8),
          Text(card['title'] as String, style: displayStyle(17, color: c.indigo900)),
          if ((card['summary'] as String).isNotEmpty) ...[
            const SizedBox(height: 4),
            Text(card['summary'] as String, style: textStyle(13, color: c.ink2)),
          ],
          if ((card['effects'] as List).isNotEmpty) EffectBox(lines: (card['effects'] as List).cast<String>()),
          if (reasons.isNotEmpty) ...[
            const SizedBox(height: 10),
            Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [
              Text('Why ${level == 'high' ? 'high' : level} risk:', style: textStyle(12, color: c.muted)),
              for (final r in reasons) AgxChip(r),
            ]),
          ],
          const SizedBox(height: 8),
          Text('Waiting since ${card['since']}${card['expires'] != null ? ' · until ${card['expires']}; nothing happens if you don\'t answer' : ''}', style: textStyle(12, color: c.muted)),
        ]),
      ),
      const SizedBox(height: 12),
      if (_done != null)
        Notice(_done!, tone: Tone.ok)
      else ...[
        if (_error != null) ...[Notice(_error!, tone: Tone.risk), const SizedBox(height: 10)],
        if (can['approve_with_code'] == true) ...[
          AgxButton('Approve with authenticator code', kind: AgxButtonKind.primary, large: true, expand: true, onPressed: _busy ? null : _approve),
          const SizedBox(height: 8),
          AgxButton('Approve with passkey in the console', expand: true, onPressed: _busy ? null : _openInConsole),
        ] else
          AgxButton('Approve with passkey in the console', kind: AgxButtonKind.primary, large: true, expand: true, onPressed: _busy ? null : _openInConsole),
        const SizedBox(height: 8),
        if (can['decline'] == true) AgxButton('Decline', kind: AgxButtonKind.ghost, expand: true, onPressed: _busy ? null : _decline),
      ],
      if (_done != null) ...[
        const SizedBox(height: 10),
        AgxButton('Next decision', expand: true, onPressed: () => widget.onDecided()),
      ],
    ]);
  }
}

/// The approval sheet on the phone (phone.html `.p-sheet`): what exactly is signed, then six
/// digits from the authenticator app, bound to that fingerprint. It runs once.
class ApproveSheet extends StatefulWidget {
  const ApproveSheet({super.key, required this.api, required this.card});

  final OwnerApi api;
  final Json card;

  @override
  State<ApproveSheet> createState() => _ApproveSheetState();
}

class _ApproveSheetState extends State<ApproveSheet> {
  final _code = TextEditingController();
  bool _busy = false;
  String? _error;

  Future<void> _sign() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final r = await widget.api.approve(widget.card['id'] as String, _code.text);
      final receipt = (r['receipt'] as String?)?.replaceFirst('sha256:', '');
      if (!mounted) return;
      Navigator.of(context).pop('Approved with your authenticator code${receipt == null ? '' : ' · receipt r-${receipt.substring(0, 4)}…${receipt.substring(receipt.length - 2)}'}. It runs once.');
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    const white = Colors.white;
    return Padding(
      padding: EdgeInsets.fromLTRB(20, 20, 20, 20 + MediaQuery.viewInsetsOf(context).bottom),
      child: Column(mainAxisSize: MainAxisSize.min, children: [
        const Icon(Icons.fingerprint, size: 44, color: white),
        const SizedBox(height: 10),
        Text("Confirm it's you", style: displayStyle(20, color: white)),
        const SizedBox(height: 6),
        Text(widget.card['title'] as String, textAlign: TextAlign.center, style: textStyle(13, color: white.withValues(alpha: 0.85))),
        const SizedBox(height: 10),
        Container(
          padding: const EdgeInsets.all(10),
          decoration: BoxDecoration(color: white.withValues(alpha: 0.08), borderRadius: BorderRadius.circular(10)),
          child: Text('Fingerprint ${widget.card['fingerprint_short']}… · expires ${widget.card['expires']}', style: monoStyle(12, color: white.withValues(alpha: 0.85))),
        ),
        const SizedBox(height: 14),
        TextField(
          controller: _code,
          autofocus: true,
          keyboardType: TextInputType.number,
          inputFormatters: [FilteringTextInputFormatter.digitsOnly, LengthLimitingTextInputFormatter(6)],
          textAlign: TextAlign.center,
          style: monoStyle(26, color: const Color(0xFF1F1D3D)),
          decoration: const InputDecoration(hintText: '6 digits from your authenticator app'),
          onSubmitted: (_) => _sign(),
        ),
        if (_error != null) ...[const SizedBox(height: 10), Text(_error!, textAlign: TextAlign.center, style: textStyle(13, color: const Color(0xFFFF9B93)))],
        const SizedBox(height: 14),
        Row(children: [
          Expanded(child: AgxButton('Cancel', onPressed: _busy ? null : () => Navigator.of(context).pop())),
          const SizedBox(width: 10),
          Expanded(child: AgxButton(_busy ? 'Approving…' : 'Approve', kind: AgxButtonKind.primary, onPressed: _busy ? null : _sign)),
        ]),
      ]),
    );
  }
}
