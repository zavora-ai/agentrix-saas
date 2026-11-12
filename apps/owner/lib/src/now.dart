import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';

/// Now (phone.html, second phone): greeting and team mark, the brief, the Needs you stack
/// (swipe to snooze, tap to open, never swipe to approve), and Freeze when it's on.
class NowScreen extends StatefulWidget {
  const NowScreen({super.key, required this.api, required this.onOpen, required this.onUnpaired, required this.onLoaded});

  final OwnerApi api;
  final ValueChanged<String> onOpen;
  final VoidCallback onUnpaired;
  final ValueChanged<Json> onLoaded;

  @override
  State<NowScreen> createState() => _NowScreenState();
}

class _NowScreenState extends State<NowScreen> {
  Json? _now;
  String? _error;
  final _snoozed = <String>{};

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final now = await widget.api.now();
      setState(() {
        _now = now;
        _error = null;
      });
      widget.onLoaded(now);
    } on ApiError catch (e) {
      if (e.unpaired) return widget.onUnpaired();
      setState(() => _error = e.message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final now = _now;
    if (now == null) {
      return Center(child: _error == null ? const CircularProgressIndicator() : Padding(padding: const EdgeInsets.all(20), child: Notice(_error!, tone: Tone.risk)));
    }
    final site = now['site'] as Json;
    final brief = now['brief'] as Json;
    final cards = (now['needs_you'] as List).cast<Json>().where((c) => !_snoozed.contains(c['id'])).toList();
    final waiting = (now['waiting'] as num).toInt();
    final frozen = now['frozen'] as Json?;
    final lines = (brief['lines'] as List).cast<String>();
    final spend = spendLine(brief['spend']);
    final name = site['name'] as String;
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 24),
        children: [
          Row(children: [
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(now['greeting'] as String, style: displayStyle(20, color: c.indigo900)),
                Text('$name${site['sample'] == true ? ' · Sample' : ''}${frozen != null ? ' · frozen' : ''}', style: textStyle(12, color: c.muted)),
              ]),
            ),
            TeamMark(size: 32, letter: name.isEmpty ? 'A' : name[0], waiting: waiting > 0),
          ]),
          const SizedBox(height: 10),
          if (frozen != null) ...[
            Notice('Every WebAgent has been frozen since ${frozen['at']}. Your site keeps serving its signed release. Unfreeze in the console with your passkey or authenticator code.', tone: Tone.risk),
            const SizedBox(height: 10),
          ],
          BriefCard(
            child: Text.rich(TextSpan(style: textStyle(13, color: c.ink2), children: [
              TextSpan(text: '${brief['headline']} ', style: textStyle(13, weight: FontWeight.w700, color: c.indigo900)),
              for (final l in lines) TextSpan(text: '$l '),
              if (spend != null) TextSpan(text: spend),
            ])),
          ),
          const SizedBox(height: 14),
          SectionLabel('Needs you · $waiting', trailing: waiting == 0 ? null : 'about ${waiting < 3 ? 1 : (waiting / 2).ceil()} minute${waiting < 3 ? '' : 's'}'),
          const SizedBox(height: 8),
          if (cards.isEmpty)
            AgxCard(flat: true, child: Text('Nothing needs you. New decisions appear here, and a notification says only that one is waiting.', style: textStyle(13, color: c.ink2)))
          else ...[
            _Stack(
              card: cards.first,
              onOpen: () => widget.onOpen(cards.first['id'] as String),
              onSnooze: () => setState(() => _snoozed.add(cards.first['id'] as String)),
            ),
            const SizedBox(height: 6),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
                Text('← snooze', style: textStyle(10.5, weight: FontWeight.w600, color: c.muted)),
                Text('tap to open', style: textStyle(10.5, weight: FontWeight.w600, color: c.muted)),
                Text('never swipe to approve', style: textStyle(10.5, weight: FontWeight.w600, color: c.muted)),
              ]),
            ),
          ],
          if (_snoozed.isNotEmpty) ...[
            const SizedBox(height: 10),
            Text("${_snoozed.length} snoozed until you refresh. Snoozing changes when you're reminded, never the decision.", style: textStyle(12, color: c.muted)),
          ],
        ],
      ),
    );
  }
}

/// The brief's AI spend line: this month against the monthly AI budget, in the site's currency,
/// as the server words it ("AI spend KSh 38 of KSh 3,870 this month."). Null when the server
/// couldn't read it, so the line is left out rather than guessed.
String? spendLine(Object? spend) {
  final words = spend is Map ? spend['words'] : null;
  return words is String && words.trim().isNotEmpty ? 'AI spend ${words.trim()}.' : null;
}

/// The top card of the stack, with the two cards behind it (`.p-stack`).
class _Stack extends StatelessWidget {
  const _Stack({required this.card, required this.onOpen, required this.onSnooze});

  final Json card;
  final VoidCallback onOpen;
  final VoidCallback onSnooze;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final agent = card['agent'] as String;
    return Padding(
      padding: const EdgeInsets.only(bottom: 14),
      child: Stack(clipBehavior: Clip.none, children: [
        Positioned(left: 18, right: 18, bottom: -14, height: 20, child: Opacity(opacity: 0.6, child: _Behind(c: c))),
        Positioned(left: 10, right: 10, bottom: -7, height: 20, child: _Behind(c: c)),
        Dismissible(
          key: ValueKey(card['id']),
          direction: DismissDirection.endToStart,
          onDismissed: (_) => onSnooze(),
          background: Container(alignment: Alignment.centerRight, padding: const EdgeInsets.only(right: 16), child: Text('Snooze', style: textStyle(13, weight: FontWeight.w700, color: c.muted))),
          child: AgxCard(
            onTap: onOpen,
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Row(children: [
                AgentAvatar(name: agent, size: 26),
                const SizedBox(width: 8),
                Expanded(child: Text('${agent.replaceFirst('The ', '')} asks', style: textStyle(12, weight: FontWeight.w700, color: c.indigo900))),
                AgxChip((card['risk_words'] as String).replaceAll(' risk', ''), tone: toneForRisk(card['risk'] as String)),
              ]),
              const SizedBox(height: 8),
              Text(card['title'] as String, style: displayStyle(16, color: c.indigo900)),
              const SizedBox(height: 4),
              Text('Waiting since ${card['since']}${card['money'] == true ? ' · moves money' : ''}', style: textStyle(12, color: c.ink2)),
            ]),
          ),
        ),
      ]),
    );
  }
}

class _Behind extends StatelessWidget {
  const _Behind({required this.c});

  final AgxColors c;

  @override
  Widget build(BuildContext context) => Container(
        decoration: BoxDecoration(
          color: c.paper,
          borderRadius: const BorderRadius.vertical(bottom: Radius.circular(14)),
          border: Border(left: BorderSide(color: c.line), right: BorderSide(color: c.line), bottom: BorderSide(color: c.line)),
        ),
      );
}
