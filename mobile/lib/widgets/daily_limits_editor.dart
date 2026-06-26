import 'package:flutter/material.dart';
import '../l10n.dart';
import '../models.dart';
import '../utils.dart';

class DailyLimitsEditor extends StatefulWidget {
  final List<DailyLimit> initialLimits;
  final Future<void> Function(List<DailyLimit>) onSave;

  const DailyLimitsEditor({
    super.key,
    required this.initialLimits,
    required this.onSave,
  });

  @override
  State<DailyLimitsEditor> createState() => _DailyLimitsEditorState();
}

class _DailyLimitsEditorState extends State<DailyLimitsEditor> {
  late List<int?> _minutes; // index = day_of_week (0=Mon), null = no limit
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _minutes = List.filled(7, null);
    for (final l in widget.initialLimits) {
      _minutes[l.dayOfWeek] = l.allowedMinutes;
    }
  }

  void _adjust(int day, int delta) {
    setState(() {
      final current = _minutes[day] ?? 0;
      final next = (current + delta).clamp(0, 1440);
      _minutes[day] = next;
    });
  }

  void _block(int day) => setState(() => _minutes[day] = 0);
  void _clear(int day) => setState(() => _minutes[day] = null);

  Future<void> _save() async {
    setState(() => _saving = true);
    final limits = <DailyLimit>[];
    for (int i = 0; i < 7; i++) {
      if (_minutes[i] != null) {
        limits.add(DailyLimit(dayOfWeek: i, allowedMinutes: _minutes[i]));
      }
    }
    await widget.onSave(limits);
    if (mounted) setState(() => _saving = false);
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(l.dailyLimits, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 4),
            Text(
              l.dailyLimitsDesc,
              style: Theme.of(context)
                  .textTheme
                  .bodySmall
                  ?.copyWith(color: cs.onSurfaceVariant),
            ),
            const SizedBox(height: 16),
            ...List.generate(7, (i) => _DayRow(
              day: i,
              minutes: _minutes[i],
              onAdjust: (d) => _adjust(i, d),
              onBlock: () => _block(i),
              onClear: () => _clear(i),
            )),
            const SizedBox(height: 12),
            SizedBox(
              width: double.infinity,
              child: FilledButton(
                onPressed: _saving ? null : _save,
                child: _saving
                    ? const SizedBox(
                        height: 18, width: 18, child: CircularProgressIndicator(strokeWidth: 2))
                    : Text(l.saveLimits),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _DayRow extends StatelessWidget {
  final int day;
  final int? minutes;
  final void Function(int delta) onAdjust;
  final VoidCallback onBlock;
  final VoidCallback onClear;

  const _DayRow({
    required this.day,
    required this.minutes,
    required this.onAdjust,
    required this.onBlock,
    required this.onClear,
  });

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final isBlocked = minutes == 0;
    final hasLimit = minutes != null;

    String limitLabel;
    Color labelColor;
    if (!hasLimit) {
      limitLabel = l.noLimitShort;
      labelColor = cs.onSurfaceVariant;
    } else if (isBlocked) {
      limitLabel = l.blocked;
      labelColor = cs.error;
    } else {
      limitLabel = formatMinutes(minutes!);
      labelColor = cs.onSurface;
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          SizedBox(
            width: 40,
            child: Text(
              l.dayShort(day),
              style: Theme.of(context)
                  .textTheme
                  .bodyMedium
                  ?.copyWith(fontWeight: FontWeight.w600),
            ),
          ),
          Expanded(
            child: Text(limitLabel, style: TextStyle(color: labelColor)),
          ),
          if (hasLimit && !isBlocked) ...[
            _SmallButton(
              label: '−15m',
              onTap: () => onAdjust(-15),
              color: cs.errorContainer,
              textColor: cs.onErrorContainer,
            ),
            const SizedBox(width: 4),
            _SmallButton(
              label: '+15m',
              onTap: () => onAdjust(15),
              color: cs.primaryContainer,
              textColor: cs.onPrimaryContainer,
            ),
            const SizedBox(width: 4),
            _SmallButton(
              label: l.block,
              onTap: onBlock,
              color: cs.surfaceContainerHighest,
              textColor: cs.onSurfaceVariant,
            ),
            const SizedBox(width: 4),
            _SmallButton(
              label: l.clear,
              onTap: onClear,
              color: cs.surfaceContainerHighest,
              textColor: cs.onSurfaceVariant,
            ),
          ] else if (isBlocked) ...[
            _SmallButton(
              label: '+15m',
              onTap: () => onAdjust(15),
              color: cs.primaryContainer,
              textColor: cs.onPrimaryContainer,
            ),
            const SizedBox(width: 4),
            _SmallButton(
              label: l.clear,
              onTap: onClear,
              color: cs.surfaceContainerHighest,
              textColor: cs.onSurfaceVariant,
            ),
          ] else ...[
            _SmallButton(
              label: l.addLimit,
              onTap: () => onAdjust(60),
              color: cs.primaryContainer,
              textColor: cs.onPrimaryContainer,
            ),
            const SizedBox(width: 4),
            _SmallButton(
              label: l.block,
              onTap: onBlock,
              color: cs.errorContainer,
              textColor: cs.onErrorContainer,
            ),
          ],
        ],
      ),
    );
  }
}

class _SmallButton extends StatelessWidget {
  final String label;
  final VoidCallback onTap;
  final Color color;
  final Color textColor;

  const _SmallButton({
    required this.label,
    required this.onTap,
    required this.color,
    required this.textColor,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Text(label, style: TextStyle(color: textColor, fontSize: 12)),
      ),
    );
  }
}
