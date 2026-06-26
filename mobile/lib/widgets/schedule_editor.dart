import 'package:flutter/material.dart';
import '../l10n.dart';
import '../models.dart';
import '../utils.dart';

class ScheduleEditor extends StatefulWidget {
  final List<Schedule> initialSchedules;
  final Future<void> Function(List<Schedule>) onSave;

  const ScheduleEditor({
    super.key,
    required this.initialSchedules,
    required this.onSave,
  });

  @override
  State<ScheduleEditor> createState() => _ScheduleEditorState();
}

class _ScheduleEditorState extends State<ScheduleEditor> {
  late List<Schedule> _schedules;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _schedules = List.from(widget.initialSchedules);
  }

  void _delete(int index) => setState(() => _schedules.removeAt(index));

  void _update(int index, Schedule s) =>
      setState(() => _schedules[index] = s);

  Future<void> _addNew() async {
    final result = await _showScheduleDialog(context, null);
    if (result != null) setState(() => _schedules.add(result));
  }

  Future<void> _edit(int index) async {
    final result = await _showScheduleDialog(context, _schedules[index]);
    if (result != null) _update(index, result);
  }

  Future<void> _save() async {
    setState(() => _saving = true);
    await widget.onSave(_schedules);
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
            Text(l.timeWindows, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 4),
            Text(
              l.timeWindowsDesc,
              style: Theme.of(context)
                  .textTheme
                  .bodySmall
                  ?.copyWith(color: cs.onSurfaceVariant),
            ),
            const SizedBox(height: 12),
            if (_schedules.isEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 8),
                child: Text(
                  l.noWindows,
                  style: TextStyle(color: cs.onSurfaceVariant),
                ),
              )
            else
              ...List.generate(_schedules.length, (i) {
                final s = _schedules[i];
                return ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: Chip(label: Text(l.dayShort(s.dayOfWeek))),
                  title: Text('${s.startTime} – ${s.endTime}'),
                  trailing: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      IconButton(
                        icon: const Icon(Icons.edit_outlined),
                        onPressed: () => _edit(i),
                      ),
                      IconButton(
                        icon: Icon(Icons.delete_outline, color: cs.error),
                        onPressed: () => _delete(i),
                      ),
                    ],
                  ),
                );
              }),
            const SizedBox(height: 8),
            OutlinedButton.icon(
              onPressed: _addNew,
              icon: const Icon(Icons.add),
              label: Text(l.addWindow),
            ),
            const SizedBox(height: 12),
            SizedBox(
              width: double.infinity,
              child: FilledButton(
                onPressed: _saving ? null : _save,
                child: _saving
                    ? const SizedBox(
                        height: 18, width: 18, child: CircularProgressIndicator(strokeWidth: 2))
                    : Text(l.saveWindows),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

Future<Schedule?> _showScheduleDialog(BuildContext context, Schedule? existing) async {
  int selectedDay = existing?.dayOfWeek ?? 0;
  TimeOfDay startTime = existing != null
      ? stringToTimeOfDay(existing.startTime)
      : const TimeOfDay(hour: 8, minute: 0);
  TimeOfDay endTime = existing != null
      ? stringToTimeOfDay(existing.endTime)
      : const TimeOfDay(hour: 20, minute: 0);

  return showDialog<Schedule>(
    context: context,
    builder: (ctx) => StatefulBuilder(
      builder: (ctx, setStateDialog) {
        final l = AppLocalizations.of(ctx);
        return AlertDialog(
          title: Text(existing == null ? l.addTimeWindow : l.editTimeWindow),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              DropdownButtonFormField<int>(
                initialValue: selectedDay,
                decoration: InputDecoration(
                  labelText: l.dayLabel,
                  border: const OutlineInputBorder(),
                ),
                items: List.generate(
                  7,
                  (i) => DropdownMenuItem(value: i, child: Text(l.dayFull(i))),
                ),
                onChanged: (v) => setStateDialog(() => selectedDay = v!),
              ),
              const SizedBox(height: 16),
              Row(
                children: [
                  Expanded(
                    child: _TimeTile(
                      label: l.start,
                      time: startTime,
                      onTap: () async {
                        final t = await showTimePicker(
                          context: ctx,
                          initialTime: startTime,
                          builder: (context, child) => MediaQuery(
                            data: MediaQuery.of(context)
                                .copyWith(alwaysUse24HourFormat: true),
                            child: child!,
                          ),
                        );
                        if (t != null) setStateDialog(() => startTime = t);
                      },
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: _TimeTile(
                      label: l.end,
                      time: endTime,
                      onTap: () async {
                        final t = await showTimePicker(
                          context: ctx,
                          initialTime: endTime,
                          builder: (context, child) => MediaQuery(
                            data: MediaQuery.of(context)
                                .copyWith(alwaysUse24HourFormat: true),
                            child: child!,
                          ),
                        );
                        if (t != null) setStateDialog(() => endTime = t);
                      },
                    ),
                  ),
                ],
              ),
            ],
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(ctx), child: Text(l.cancel)),
            FilledButton(
              onPressed: () => Navigator.pop(
                ctx,
                Schedule(
                  id: existing?.id,
                  dayOfWeek: selectedDay,
                  startTime: timeOfDayToString(startTime),
                  endTime: timeOfDayToString(endTime),
                ),
              ),
              child: Text(l.save),
            ),
          ],
        );
      },
    ),
  );
}

class _TimeTile extends StatelessWidget {
  final String label;
  final TimeOfDay time;
  final VoidCallback onTap;

  const _TimeTile({required this.label, required this.time, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(8),
      child: Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          border: Border.all(color: Theme.of(context).colorScheme.outline),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(label,
                style: Theme.of(context)
                    .textTheme
                    .bodySmall
                    ?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant)),
            Text(
              timeOfDayToString(time),
              style: Theme.of(context).textTheme.titleMedium,
            ),
          ],
        ),
      ),
    );
  }
}
