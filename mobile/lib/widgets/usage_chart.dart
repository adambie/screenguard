import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:intl/intl.dart';
import '../auth_provider.dart';
import '../l10n.dart';
import '../models.dart';
import '../utils.dart';

class UsageChart extends ConsumerStatefulWidget {
  final String profileId;
  const UsageChart({super.key, required this.profileId});

  @override
  ConsumerState<UsageChart> createState() => _UsageChartState();
}

class _UsageChartState extends ConsumerState<UsageChart> {
  int _weekOffset = 0;
  List<UsageEntry>? _usage;
  bool _loading = false;

  DateTime get _weekStart {
    final now = DateTime.now();
    final monday = now.subtract(Duration(days: now.weekday - 1));
    return DateTime(monday.year, monday.month, monday.day)
        .add(Duration(days: _weekOffset * 7));
  }

  @override
  void initState() {
    super.initState();
    _fetch();
  }

  Future<void> _fetch() async {
    setState(() => _loading = true);
    try {
      final start = _weekStart;
      final end = start.add(const Duration(days: 6));
      final from = '${start.year}-${start.month.toString().padLeft(2, '0')}-${start.day.toString().padLeft(2, '0')}';
      final to = '${end.year}-${end.month.toString().padLeft(2, '0')}-${end.day.toString().padLeft(2, '0')}';
      final data = await ref
          .read(apiClientProvider)
          .get('/profiles/${widget.profileId}/usage?from=$from&to=$to') as Map<String, dynamic>;
      final entries = ((data['usage'] as List?) ?? [])
          .map((u) => UsageEntry.fromJson(u as Map<String, dynamic>))
          .toList();
      if (mounted) setState(() => _usage = entries);
    } catch (_) {
      if (mounted) setState(() => _usage = []);
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  void _prevWeek() {
    setState(() => _weekOffset--);
    _fetch();
  }

  void _nextWeek() {
    if (_weekOffset < 0) {
      setState(() => _weekOffset++);
      _fetch();
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final start = _weekStart;
    final end = start.add(const Duration(days: 6));

    String fmtDate(DateTime d) {
      try {
        return DateFormat('d MMM', l.locale.languageCode).format(d);
      } catch (_) {
        const months = ['Jan','Feb','Mar','Apr','May','Jun',
                        'Jul','Aug','Sep','Oct','Nov','Dec'];
        return '${d.day} ${months[d.month - 1]}';
      }
    }

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text(l.chartUsage, style: Theme.of(context).textTheme.titleMedium),
                const Spacer(),
                IconButton(
                  icon: const Icon(Icons.chevron_left),
                  onPressed: _loading ? null : _prevWeek,
                ),
                Text(
                  '${fmtDate(start)} – ${fmtDate(end)}',
                  style: Theme.of(context).textTheme.bodySmall,
                ),
                IconButton(
                  icon: const Icon(Icons.chevron_right),
                  onPressed: (_weekOffset >= 0 || _loading) ? null : _nextWeek,
                ),
              ],
            ),
            const SizedBox(height: 16),
            if (_loading)
              const SizedBox(height: 120, child: Center(child: CircularProgressIndicator()))
            else
              _buildChart(cs, start, l),
          ],
        ),
      ),
    );
  }

  Widget _buildChart(ColorScheme cs, DateTime weekStart, AppLocalizations l) {
    final today = DateTime.now();
    final usage = _usage ?? [];

    double maxMinutes = 60;
    for (final e in usage) {
      if (e.usedMinutes > maxMinutes) maxMinutes = e.usedMinutes.toDouble();
      if ((e.limitMinutes ?? 0) > maxMinutes) maxMinutes = e.limitMinutes!.toDouble();
    }

    final groups = List.generate(7, (i) {
      final day = weekStart.add(Duration(days: i));
      final dateStr =
          '${day.year}-${day.month.toString().padLeft(2, '0')}-${day.day.toString().padLeft(2, '0')}';
      final entry = usage.firstWhere((e) => e.date == dateStr,
          orElse: () => UsageEntry(date: dateStr, usedMinutes: 0, adjustmentsMinutes: 0));
      final isFuture = day.isAfter(today);
      final isToday = day.year == today.year &&
          day.month == today.month &&
          day.day == today.day;

      final barColor = isFuture
          ? cs.surfaceContainerHighest
          : isToday
              ? cs.primary
              : cs.primaryContainer;

      return BarChartGroupData(
        x: i,
        barRods: [
          BarChartRodData(
            toY: entry.usedMinutes > 0 ? entry.usedMinutes.toDouble() : 0,
            color: barColor,
            width: 28,
            borderRadius: BorderRadius.circular(4),
            backDrawRodData: BackgroundBarChartRodData(
              show: entry.limitMinutes != null,
              toY: entry.limitMinutes?.toDouble() ?? 0,
              color: cs.surfaceContainerHighest,
            ),
          ),
        ],
      );
    });

    return SizedBox(
      height: 150,
      child: BarChart(
        BarChartData(
          maxY: maxMinutes * 1.2,
          barGroups: groups,
          gridData: FlGridData(show: false),
          borderData: FlBorderData(show: false),
          titlesData: FlTitlesData(
            leftTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
            rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
            topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
            bottomTitles: AxisTitles(
              sideTitles: SideTitles(
                showTitles: true,
                getTitlesWidget: (v, meta) {
                  final idx = v.toInt().clamp(0, 6);
                  return Text(
                    l.dayLetter(idx),
                    style: TextStyle(
                      fontSize: 11,
                      color: cs.onSurfaceVariant,
                      fontWeight: idx < 5 ? FontWeight.normal : FontWeight.w600,
                    ),
                  );
                },
              ),
            ),
          ),
          barTouchData: BarTouchData(
            touchTooltipData: BarTouchTooltipData(
              getTooltipItem: (group, groupIndex, rod, rodIndex) {
                final day = weekStart.add(Duration(days: group.x));
                final dateStr =
                    '${day.year}-${day.month.toString().padLeft(2, '0')}-${day.day.toString().padLeft(2, '0')}';
                final entry = usage.firstWhere((e) => e.date == dateStr,
                    orElse: () =>
                        UsageEntry(date: dateStr, usedMinutes: 0, adjustmentsMinutes: 0));
                return BarTooltipItem(
                  formatMinutes(entry.usedMinutes),
                  TextStyle(color: cs.onPrimary, fontWeight: FontWeight.bold),
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}
