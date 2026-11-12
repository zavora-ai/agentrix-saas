import 'package:flutter/material.dart';

import 'theme.dart';

/// How a button looks (the concepts' `.btn`, `.btn-primary`, `.btn-ghost`, `.btn-coral`).
enum AgxButtonKind { primary, plain, ghost, danger }

/// A button: 12 px radius, DM Sans 700 (concept/styles.css `.btn`).
class AgxButton extends StatelessWidget {
  const AgxButton(this.label, {super.key, this.onPressed, this.kind = AgxButtonKind.plain, this.small = false, this.large = false, this.expand = false, this.icon});

  final String label;
  final VoidCallback? onPressed;
  final AgxButtonKind kind;
  final bool small;
  final bool large;
  final bool expand;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final brand = siteBrand(context);
    final accent = brand?.accent ?? c.indigo;
    final (bg, fg, border) = switch (kind) {
      AgxButtonKind.primary => (accent, brand?.onAccent ?? Colors.white, accent),
      AgxButtonKind.plain => (brand?.bg ?? c.paper, brand?.ink ?? c.indigo900, brand?.line ?? c.lineStrong),
      AgxButtonKind.ghost => (Colors.transparent, c.ink2, Colors.transparent),
      AgxButtonKind.danger => (brand?.bg ?? c.paper, c.risk, c.risk),
    };
    final height = small ? 34.0 : (large ? 48.0 : 42.0);
    final child = Row(
      mainAxisSize: expand ? MainAxisSize.max : MainAxisSize.min,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        if (icon != null) ...[Icon(icon, size: 18, color: fg), const SizedBox(width: 8)],
        Flexible(child: Text(label, overflow: TextOverflow.ellipsis, style: textStyle(small ? 13 : 14, weight: FontWeight.w700, color: fg, height: 1))),
      ],
    );
    return Semantics(
      button: true,
      label: label,
      child: Opacity(
        opacity: onPressed == null ? 0.55 : 1,
        child: Material(
          color: bg,
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(small ? 10 : 12), side: BorderSide(color: border, width: kind == AgxButtonKind.danger ? 1.5 : 1)),
          shadowColor: accent.withValues(alpha: 0.7),
          elevation: kind == AgxButtonKind.primary && onPressed != null ? 3 : 0,
          child: InkWell(
            borderRadius: BorderRadius.circular(small ? 10 : 12),
            onTap: onPressed,
            child: ConstrainedBox(
              constraints: BoxConstraints(minHeight: height),
              child: Padding(padding: EdgeInsets.symmetric(horizontal: small ? 12 : 16), child: Center(widthFactor: expand ? null : 1, child: child)),
            ),
          ),
        ),
      ),
    );
  }
}

/// A chip's tone (`.chip-ok`, `.chip-wait`, `.chip-risk`).
enum Tone { ok, wait, risk, neutral }

/// The tone for a card's risk level.
Tone toneForRisk(String level) => switch (level) {
      'low' => Tone.ok,
      'medium' => Tone.wait,
      'high' => Tone.risk,
      _ => Tone.neutral,
    };

/// A small rounded label.
class AgxChip extends StatelessWidget {
  const AgxChip(this.label, {super.key, this.tone = Tone.neutral});

  final String label;
  final Tone tone;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final (bg, fg) = switch (tone) {
      Tone.ok => (c.okBg, c.ok),
      Tone.wait => (c.waitBg, c.wait),
      Tone.risk => (c.riskBg, c.risk),
      Tone.neutral => (c.surface2, c.ink2),
    };
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
      decoration: BoxDecoration(color: bg, borderRadius: BorderRadius.circular(999)),
      child: Text(label, style: textStyle(12, weight: FontWeight.w600, color: fg, height: 1.3)),
    );
  }
}

/// A phone card (concept/console/phone.html `.p-card`): paper, line, 16 px radius, a soft shadow.
class AgxCard extends StatelessWidget {
  const AgxCard({super.key, required this.child, this.onTap, this.padding = const EdgeInsets.all(12), this.flat = false});

  final Widget child;
  final VoidCallback? onTap;
  final EdgeInsets padding;
  final bool flat;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final brand = siteBrand(context);
    return Container(
      decoration: BoxDecoration(
        color: brand?.bg ?? c.paper,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: brand?.line ?? c.line),
        boxShadow: flat ? null : [BoxShadow(color: c.indigo900.withValues(alpha: 0.12), blurRadius: 24, offset: const Offset(0, 8), spreadRadius: -10)],
      ),
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(borderRadius: BorderRadius.circular(16), onTap: onTap, child: Padding(padding: padding, child: child)),
      ),
    );
  }
}

/// The brief (`.p-brief`): a coral rule on the left.
class BriefCard extends StatelessWidget {
  const BriefCard({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: c.paper,
        borderRadius: BorderRadius.circular(16),
        border: Border(left: BorderSide(color: c.coral, width: 4), top: BorderSide(color: c.line), right: BorderSide(color: c.line), bottom: BorderSide(color: c.line)),
      ),
      child: child,
    );
  }
}

/// "If you approve, exactly this happens" (`.effect-box`).
class EffectBox extends StatelessWidget {
  const EffectBox({super.key, required this.lines});

  final List<String> lines;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    return Container(
      width: double.infinity,
      margin: const EdgeInsets.only(top: 10),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(color: c.indigo50, borderRadius: BorderRadius.circular(12), border: Border.all(color: c.indigo100)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('If you approve, exactly this happens', style: textStyle(12, weight: FontWeight.w700, color: c.indigo900)),
          const SizedBox(height: 6),
          for (final l in lines)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Padding(padding: const EdgeInsets.only(top: 7, right: 8), child: Container(width: 5, height: 5, decoration: BoxDecoration(color: c.indigo, shape: BoxShape.circle))),
                Expanded(child: Text(l, style: textStyle(13, color: c.ink2))),
              ]),
            ),
        ],
      ),
    );
  }
}

/// The Chief WebAgent's team mark (console.css `.orb`): a dark squircle with the team's four
/// dots and the site's initial. A dot on the corner when something waits.
class TeamMark extends StatelessWidget {
  const TeamMark({super.key, this.size = 44, this.letter = 'K', this.waiting = false, this.listening = false});

  final double size;
  final String letter;
  final bool waiting;
  final bool listening;

  @override
  Widget build(BuildContext context) {
    const ink = Color(0xFF2B1D14);
    const paper = Color(0xFFFBF6EF);
    const dots = [Color(0xFF6B8CF0), Color(0xFFE3607B), Color(0xFFE0A03A), Color(0xFF8FA3CF)];
    final d = size * 0.096;
    return Semantics(
      label: 'Chief WebAgent',
      child: SizedBox(
        width: size,
        height: size,
        child: Stack(clipBehavior: Clip.none, children: [
          AnimatedContainer(
            duration: const Duration(milliseconds: 300),
            width: size,
            height: size,
            decoration: BoxDecoration(
              color: ink,
              borderRadius: BorderRadius.circular(size * 0.3),
              boxShadow: [
                BoxShadow(color: ink.withValues(alpha: 0.75), blurRadius: 16, offset: const Offset(0, 6), spreadRadius: -9),
                if (listening) const BoxShadow(color: Color(0x66E3607B), blurRadius: 0, spreadRadius: 4),
              ],
            ),
            alignment: const Alignment(0, -0.25),
            child: Text(letter, style: TextStyle(fontFamily: 'Fraunces', package: kitPackage, fontSize: size * 0.42, color: paper, fontWeight: FontWeight.w600, height: 1)),
          ),
          for (var i = 0; i < 4; i++)
            Positioned(
              left: size * (0.32 + 0.12 * i) - d / 2,
              top: size * 0.84 - d / 2,
              child: Container(width: d, height: d, decoration: BoxDecoration(color: dots[i], shape: BoxShape.circle)),
            ),
          if (waiting)
            Positioned(
              top: -3,
              right: -3,
              child: Container(width: 9, height: 9, decoration: BoxDecoration(color: const Color(0xFFB26B00), shape: BoxShape.circle, border: Border.all(color: Colors.white, width: 2))),
            ),
        ]),
      ),
    );
  }
}

/// The colours of a WebAgent's avatar, by role (concept/styles.css `.av-*`).
List<Color> avatarColors(String role) => switch (role.toLowerCase()) {
      'concierge' => const [Color(0xFFB72D50), Color(0xFFE3607B)],
      'sales' => const [Color(0xFF0F6B62), Color(0xFF22A394)],
      'editor' => const [Color(0xFF7A4B00), Color(0xFFC58016)],
      'cashier' => const [Color(0xFF11734A), Color(0xFF2FB07A)],
      'guardian' => const [Color(0xFF45597F), Color(0xFF6E86B5)],
      'reputation' => const [Color(0xFF5B3A8E), Color(0xFF8C6BC4)],
      'caretaker' => const [Color(0xFF4A4960), Color(0xFF7A7990)],
      _ => const [Color(0xFF38356D), Color(0xFF5E5AA8)],
    };

/// A WebAgent's tile: initials on its colour.
class AgentAvatar extends StatelessWidget {
  const AgentAvatar({super.key, required this.name, this.role, this.size = 40, this.fill});

  final String name;
  final String? role;
  final double size;
  final Color? fill;

  @override
  Widget build(BuildContext context) {
    final words = name.replaceFirst(RegExp(r'^The\s+'), '').trim();
    final initials = words.isEmpty ? '·' : (words.length >= 2 ? words.substring(0, 2) : words);
    final colors = fill != null ? [fill!, fill!] : avatarColors(role ?? words);
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(gradient: LinearGradient(colors: colors, begin: Alignment.topLeft, end: Alignment.bottomRight), borderRadius: BorderRadius.circular(size * 0.3)),
      child: Text(initials, style: displayStyle(size * 0.34, color: Colors.white, height: 1)),
    );
  }
}

/// A tab in the phone's tab bar.
class PhoneTab {
  const PhoneTab(this.label, this.icon, {this.badge = 0});
  final String label;
  final IconData icon;
  final int badge;
}

/// The phone's tab bar (`.ptabs`): Now, Decisions, Live, Apps.
class PhoneTabs extends StatelessWidget {
  const PhoneTabs({super.key, required this.tabs, required this.index, required this.onTap});

  final List<PhoneTab> tabs;
  final int index;
  final ValueChanged<int> onTap;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    return Container(
      decoration: BoxDecoration(color: c.paper, border: Border(top: BorderSide(color: c.line))),
      padding: EdgeInsets.only(top: 6, bottom: 6 + MediaQuery.paddingOf(context).bottom),
      child: Row(children: [
        for (var i = 0; i < tabs.length; i++)
          Expanded(
            child: Semantics(
              selected: i == index,
              button: true,
              label: tabs[i].label,
              child: InkWell(
                onTap: () => onTap(i),
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 6),
                  child: Column(mainAxisSize: MainAxisSize.min, children: [
                    Stack(clipBehavior: Clip.none, children: [
                      Icon(tabs[i].icon, size: 22, color: i == index ? c.coralStrong : c.muted),
                      if (tabs[i].badge > 0)
                        Positioned(
                          right: -10,
                          top: -4,
                          child: Container(
                            padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
                            decoration: BoxDecoration(color: c.coralStrong, borderRadius: BorderRadius.circular(999)),
                            child: Text('${tabs[i].badge}', style: textStyle(10, weight: FontWeight.w700, color: Colors.white, height: 1.2)),
                          ),
                        ),
                    ]),
                    const SizedBox(height: 3),
                    Text(tabs[i].label, style: textStyle(10.5, weight: FontWeight.w600, color: i == index ? c.indigo900 : c.muted, height: 1.1)),
                  ]),
                ),
              ),
            ),
          ),
      ]),
    );
  }
}

/// A section label ("Needs you · 2").
class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key, this.trailing});

  final String text;
  final String? trailing;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    return Row(children: [
      Text(text, style: textStyle(13, weight: FontWeight.w700, color: c.indigo900)),
      const Spacer(),
      if (trailing != null) Text(trailing!, style: textStyle(12, color: c.muted)),
    ]);
  }
}

/// A plain-words notice: ok, waiting or a problem.
class Notice extends StatelessWidget {
  const Notice(this.text, {super.key, this.tone = Tone.neutral});

  final String text;
  final Tone tone;

  @override
  Widget build(BuildContext context) {
    final c = agx(context);
    final (bg, fg) = switch (tone) {
      Tone.ok => (c.okBg, c.ok),
      Tone.wait => (c.waitBg, c.wait),
      Tone.risk => (c.riskBg, c.risk),
      Tone.neutral => (c.surface2, c.ink2),
    };
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(color: bg, borderRadius: BorderRadius.circular(12)),
      child: Text(text, style: textStyle(13, weight: FontWeight.w500, color: fg)),
    );
  }
}
