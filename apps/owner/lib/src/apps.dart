import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';

/// Apps (phone.html, fourth phone): the WebAgents as a home screen with badges, talk to the
/// Chief WebAgent, and Freeze. Typed words follow the same grammar as voice and chat: "stop
/// everything" freezes at once; nothing is approved by words.
class AppsScreen extends StatefulWidget {
  const AppsScreen({super.key, required this.api, required this.onOpen, required this.onUnpaired});

  final OwnerApi api;
  final ValueChanged<String> onOpen;
  final VoidCallback onUnpaired;

  @override
  State<AppsScreen> createState() => _AppsScreenState();
}

class _AppsScreenState extends State<AppsScreen> {
  List<Json>? _team;
  String? _error;
  final _said = TextEditingController();
  String? _reply;
  bool _asking = false;
  String? _frozenWords;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final t = await widget.api.webagents();
      setState(() {
        _team = t;
        _error = null;
      });
    } on ApiError catch (e) {
      if (e.unpaired) return widget.onUnpaired();
      setState(() => _error = e.message);
    }
  }

  Future<void> _ask() async {
    final said = _said.text.trim();
    if (said.isEmpty) return;
    setState(() {
      _asking = true;
      _reply = null;
    });
    try {
      final r = await widget.api.ask(said);
      setState(() => _reply = r['reply'] as String?);
      final action = r['action'];
      if (action is Json && action['do'] == 'open_card') widget.onOpen(action['card'] as String);
      _said.clear();
    } on ApiError catch (e) {
      setState(() => _reply = e.message);
    } finally {
      setState(() => _asking = false);
    }
  }

  Future<void> _freeze() async {
    final c = agx(context);
    final sure = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Freeze every WebAgent?', style: displayStyle(18, color: c.indigo900)),
        content: Text('Your site keeps serving its signed release. Nothing is sent, published or paid until you unfreeze in the console with your passkey or authenticator code.', style: textStyle(14, color: c.ink2)),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text('Keep working')),
          TextButton(onPressed: () => Navigator.pop(ctx, true), child: Text('Freeze', style: TextStyle(color: c.risk))),
        ],
      ),
    );
    if (sure != true) return;
    try {
      final r = await widget.api.freeze();
      setState(() => _frozenWords = r['message'] as String?);
    } on ApiError catch (e) {
      setState(() => _frozenWords = e.message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final team = _team ?? const <Json>[];
    final onDuty = team.where((m) => m['on_duty'] == true).toList();
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 24),
        children: [
          Text('Your WebAgents', style: displayStyle(20, color: c.indigo900)),
          Text('${onDuty.length} on the team · asleep costs nothing', style: textStyle(12, color: c.muted)),
          const SizedBox(height: 12),
          if (_error != null) Notice(_error!, tone: Tone.risk),
          if (_team == null && _error == null) const Center(child: CircularProgressIndicator()),
          GridView.count(
            crossAxisCount: 4,
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            mainAxisSpacing: 14,
            crossAxisSpacing: 6,
            childAspectRatio: 0.78,
            children: [
              for (final m in onDuty)
                Column(children: [
                  Stack(clipBehavior: Clip.none, children: [
                    AgentAvatar(name: m['name'] as String, role: m['id'] as String, size: 52, fill: _fill(m)),
                    if ((m['badge'] as num) > 0)
                      Positioned(
                        right: -4,
                        top: -4,
                        child: Container(
                          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
                          decoration: BoxDecoration(color: c.coralStrong, borderRadius: BorderRadius.circular(999)),
                          child: Text('${m['badge']}', style: textStyle(11, weight: FontWeight.w700, color: Colors.white, height: 1.3)),
                        ),
                      ),
                  ]),
                  const SizedBox(height: 6),
                  Text(m['name'] as String, maxLines: 1, overflow: TextOverflow.ellipsis, style: textStyle(11, color: c.ink2)),
                ]),
            ],
          ),
          const SizedBox(height: 14),
          AgxCard(
            flat: true,
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Row(children: [
                TeamMark(size: 44, letter: 'K', listening: _asking),
                const SizedBox(width: 12),
                Expanded(
                  child: Text.rich(TextSpan(children: [
                    TextSpan(text: 'Ask your Chief WebAgent\n', style: textStyle(14, weight: FontWeight.w700, color: c.indigo900)),
                    TextSpan(text: '"How did the weekend go?" · "What\'s next?" · "Stop everything"', style: textStyle(12, color: c.muted)),
                  ])),
                ),
              ]),
              const SizedBox(height: 10),
              Row(children: [
                Expanded(child: TextField(controller: _said, textInputAction: TextInputAction.send, onSubmitted: (_) => _ask(), decoration: const InputDecoration(hintText: 'Type a question'))),
                const SizedBox(width: 8),
                AgxButton(_asking ? 'Asking…' : 'Ask', kind: AgxButtonKind.primary, onPressed: _asking ? null : _ask),
              ]),
              if (_reply != null) ...[const SizedBox(height: 10), Text(_reply!, style: textStyle(14, color: c.ink))],
            ]),
          ),
          const SizedBox(height: 14),
          if (_frozenWords != null) ...[Notice(_frozenWords!, tone: Tone.risk), const SizedBox(height: 10)],
          AgxButton('Freeze all WebAgents', kind: AgxButtonKind.danger, large: true, expand: true, onPressed: _freeze),
        ],
      ),
    );
  }

  Color? _fill(Json m) {
    final hex = ((m['mark'] as Json?)?['color'] as Json?)?['fill'] as String?;
    if (hex == null || hex.length != 7) return null;
    return Color(int.parse('FF${hex.substring(1)}', radix: 16));
  }
}
