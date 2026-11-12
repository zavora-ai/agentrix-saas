import 'package:agentrix_api/agentrix_api.dart';
import 'package:agentrix_kit/agentrix_kit.dart';
import 'package:flutter/material.dart';

/// Live (07 §4.3): the front of house as a list, from W1's presence hub. Sitting in and taking
/// over stay in the console, where the passkey is.
class LiveScreen extends StatefulWidget {
  const LiveScreen({super.key, required this.api});

  final OwnerApi api;

  @override
  State<LiveScreen> createState() => _LiveScreenState();
}

class _LiveScreenState extends State<LiveScreen> {
  Json? _live;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final l = await widget.api.live();
      setState(() {
        _live = l;
        _error = null;
      });
    } on ApiError catch (e) {
      setState(() => _error = e.message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final live = _live;
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 24),
        children: [
          Text('Live', style: displayStyle(20, color: c.indigo900)),
          const SizedBox(height: 4),
          Text('Who is working on your site right now. Pull down to refresh.', style: textStyle(12, color: c.muted)),
          const SizedBox(height: 12),
          if (_error != null) Notice(_error!, tone: Tone.risk),
          if (live == null && _error == null) const Center(child: CircularProgressIndicator()),
          if (live != null) ...[
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
              decoration: BoxDecoration(color: c.indigo900, borderRadius: BorderRadius.circular(14)),
              child: Text(
                (live['busy'] as num) == 0 ? 'Quiet right now. Your desks answer the moment someone asks.' : '${live['busy']} at work on your site now.',
                style: textStyle(13, weight: FontWeight.w600, color: Colors.white),
              ),
            ),
            const SizedBox(height: 10),
            if ((live['frames'] as List).isEmpty)
              Text("This site's team isn't recorded yet.", style: textStyle(13, color: c.ink2)),
            for (final f in (live['frames'] as List).cast<Json>())
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: AgxCard(
                  flat: true,
                  child: Row(children: [
                    AgentAvatar(name: (f['name'] as String).replaceFirst('desk-', ''), size: 32),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                        Text(f['name'] as String, style: textStyle(14, weight: FontWeight.w700, color: c.indigo900)),
                        Text((f['label'] as String?) ?? (f['words'] as String), style: textStyle(12, color: c.ink2)),
                      ]),
                    ),
                    AgxChip('${f['state']}', tone: f['state'] == 'asleep' || f['state'] == 'idle' ? Tone.neutral : Tone.ok),
                  ]),
                ),
              ),
            const SizedBox(height: 6),
            Text('To sit in or take over a conversation, open Front of house in the console.', style: textStyle(12, color: c.muted)),
          ],
        ],
      ),
    );
  }
}
