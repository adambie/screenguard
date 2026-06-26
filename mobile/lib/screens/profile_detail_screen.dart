import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../data_providers.dart';
import '../l10n.dart';
import '../models.dart';
import '../utils.dart';
import '../widgets/daily_limits_editor.dart';
import '../widgets/schedule_editor.dart';
import '../widgets/usage_chart.dart';

class ProfileDetailScreen extends ConsumerStatefulWidget {
  final String profileId;
  const ProfileDetailScreen({super.key, required this.profileId});

  @override
  ConsumerState<ProfileDetailScreen> createState() =>
      _ProfileDetailScreenState();
}

class _ProfileDetailScreenState extends ConsumerState<ProfileDetailScreen> {
  int _adjustPending = 0;
  bool _applyingAdj = false;
  bool _locking = false;

  void _refresh() {
    ref.invalidate(profileStatusProvider(widget.profileId));
    ref.invalidate(profileDetailProvider(widget.profileId));
    ref.invalidate(dashboardProvider);
  }

  Future<void> _applyAdjustment() async {
    if (_adjustPending == 0) return;
    setState(() => _applyingAdj = true);
    try {
      final today = DateTime.now();
      final dateStr =
          '${today.year}-${today.month.toString().padLeft(2, '0')}-${today.day.toString().padLeft(2, '0')}';
      await ref.read(apiClientProvider).post(
        '/profiles/${widget.profileId}/adjustments',
        {'target_date': dateStr, 'adjustment_minutes': _adjustPending, 'reason': null},
      );
      setState(() => _adjustPending = 0);
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).timeAdjusted);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    } finally {
      if (mounted) setState(() => _applyingAdj = false);
    }
  }

  Future<void> _lockNow() async {
    final l = AppLocalizations.of(context);
    final ok = await _confirm(l.lockNowConfirmTitle, l.lockNowConfirmBody);
    if (!ok) return;
    setState(() => _locking = true);
    try {
      await ref
          .read(apiClientProvider)
          .post('/profiles/${widget.profileId}/lock-now');
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).screenTimeLocked);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    } finally {
      if (mounted) setState(() => _locking = false);
    }
  }

  Future<void> _sendMessage(String body) async {
    try {
      await ref
          .read(apiClientProvider)
          .post('/profiles/${widget.profileId}/notify', {'body': body});
      if (mounted) _snack(AppLocalizations.of(context).messageSent);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _saveLimits(List<DailyLimit> limits) async {
    try {
      await ref.read(apiClientProvider).put(
        '/profiles/${widget.profileId}/daily-limits',
        {'limits': limits.map((l) => l.toJson()).toList()},
      );
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).limitsSaved);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _saveSchedules(List<Schedule> schedules) async {
    try {
      await ref.read(apiClientProvider).put(
        '/profiles/${widget.profileId}/schedules',
        {'schedules': schedules.map((s) => s.toJson()).toList()},
      );
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).scheduleSaved);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _saveLanguage(String lang) async {
    try {
      await ref.read(apiClientProvider).patch(
        '/profiles/${widget.profileId}',
        {'language': lang},
      );
      ref.invalidate(profileDetailProvider(widget.profileId));
      if (mounted) _snack(AppLocalizations.of(context).languageSaved);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _rename(String newName) async {
    try {
      await ref.read(apiClientProvider).patch(
        '/profiles/${widget.profileId}',
        {'display_name': newName},
      );
      _refresh();
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _delete() async {
    final l = AppLocalizations.of(context);
    final ok = await _confirm(l.deleteProfileTitle, l.deleteProfileBody);
    if (!ok) return;
    try {
      await ref.read(apiClientProvider).delete('/profiles/${widget.profileId}');
      if (mounted) {
        ref.invalidate(dashboardProvider);
        Navigator.pop(context);
      }
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  void _showRenameDialog(String currentName) {
    final l = AppLocalizations.of(context);
    final ctrl = TextEditingController(text: currentName);
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.renameProfile),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          textCapitalization: TextCapitalization.words,
          decoration: const InputDecoration(border: OutlineInputBorder()),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: Text(l.cancel)),
          FilledButton(
            onPressed: () {
              final name = ctrl.text.trim();
              if (name.isNotEmpty) {
                Navigator.pop(ctx);
                _rename(name);
              }
            },
            child: Text(l.save),
          ),
        ],
      ),
    );
  }

  void _showMessageDialog() {
    final l = AppLocalizations.of(context);
    final ctrl = TextEditingController();
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.sendMessage),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          maxLines: 3,
          decoration: InputDecoration(
            hintText: l.messageToUser,
            border: const OutlineInputBorder(),
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: Text(l.cancel)),
          FilledButton(
            onPressed: () {
              final msg = ctrl.text.trim();
              if (msg.isNotEmpty) {
                Navigator.pop(ctx);
                _sendMessage(msg);
              }
            },
            child: Text(l.send),
          ),
        ],
      ),
    );
  }

  Future<bool> _confirm(String title, String body) async {
    final l = AppLocalizations.of(context);
    final result = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(title),
        content: Text(body),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: Text(l.cancel)),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text(l.confirm),
          ),
        ],
      ),
    );
    return result ?? false;
  }

  void _snack(String msg, {bool error = false}) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
      content: Text(msg),
      backgroundColor: error ? Theme.of(context).colorScheme.error : null,
    ));
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final detailAsync = ref.watch(profileDetailProvider(widget.profileId));
    final statusAsync = ref.watch(profileStatusProvider(widget.profileId));
    ref.listen(profileDetailProvider(widget.profileId), (_, state) {
      if (state.hasError && state.error is UnauthorizedException) {
        ref.read(authProvider.notifier).relogin();
      }
    });

    return Scaffold(
      appBar: AppBar(
        title: detailAsync.when(
          data: (p) => Text(p.displayName),
          loading: () => Text(l.profileTitle),
          error: (err, st) => Text(l.profileTitle),
        ),
        actions: [
          detailAsync.when(
            data: (p) => PopupMenuButton<String>(
              onSelected: (v) {
                if (v == 'rename') _showRenameDialog(p.displayName);
                if (v == 'delete') _delete();
              },
              itemBuilder: (_) => [
                PopupMenuItem(value: 'rename', child: Text(l.rename)),
                PopupMenuItem(value: 'delete', child: Text(l.delete)),
              ],
            ),
            loading: () => const SizedBox(),
            error: (err, st) => const SizedBox(),
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: () async => _refresh(),
        child: SingleChildScrollView(
          physics: const AlwaysScrollableScrollPhysics(),
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              _buildStatusCard(context, statusAsync),
              const SizedBox(height: 16),
              UsageChart(profileId: widget.profileId),
              const SizedBox(height: 16),
              detailAsync.when(
                loading: () => const Center(child: CircularProgressIndicator()),
                error: (e, _) => Card(
                  child: Padding(
                    padding: const EdgeInsets.all(16),
                    child: Text('Error loading settings: $e'),
                  ),
                ),
                data: (profile) => Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    DailyLimitsEditor(
                      initialLimits: profile.dailyLimits,
                      onSave: _saveLimits,
                    ),
                    const SizedBox(height: 16),
                    ScheduleEditor(
                      initialSchedules: profile.schedules,
                      onSave: _saveSchedules,
                    ),
                    const SizedBox(height: 16),
                    _buildLanguageCard(context, profile),
                    const SizedBox(height: 16),
                    _buildLinkedUsersCard(context, profile),
                    const SizedBox(height: 32),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildStatusCard(BuildContext context, AsyncValue<ProfileStatus> statusAsync) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: statusAsync.when(
          loading: () => const Center(
              child: Padding(
                  padding: EdgeInsets.all(16),
                  child: CircularProgressIndicator())),
          error: (e, _) => Text('Error: $e'),
          data: (status) {
            final today = status.today;
            final isLocked = today.enforce == 'lock';
            final hasLimit = today.limitMinutes != null;

            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(l.today, style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            isLocked
                                ? l.locked
                                : hasLimit
                                    ? formatMinutes(today.remainingMinutes)
                                    : l.noLimit,
                            style: Theme.of(context)
                                .textTheme
                                .displaySmall
                                ?.copyWith(
                                  color: isLocked
                                      ? cs.error
                                      : today.remainingMinutes < 15 && hasLimit
                                          ? Colors.orange
                                          : cs.primary,
                                  fontWeight: FontWeight.bold,
                                ),
                          ),
                          const SizedBox(height: 4),
                          Text(
                            hasLimit
                                ? l.usedOfLimit(
                                    formatMinutes(today.usedMinutes),
                                    formatMinutes(today.limitMinutes!),
                                  )
                                : l.usedOf(formatMinutes(today.usedMinutes)),
                            style: TextStyle(color: cs.onSurfaceVariant),
                          ),
                          if (today.adjustmentsMinutes != 0)
                            Text(
                              l.timeAdjustment(
                                '${today.adjustmentsMinutes > 0 ? '+' : ''}${formatMinutes(today.adjustmentsMinutes.abs())}',
                              ),
                              style: TextStyle(
                                color: today.adjustmentsMinutes > 0
                                    ? Colors.green
                                    : cs.error,
                                fontSize: 12,
                              ),
                            ),
                        ],
                      ),
                    ),
                    Column(
                      children: [
                        Text(
                          _adjustPending == 0
                              ? l.adjust
                              : '${_adjustPending > 0 ? '+' : '-'}${formatMinutes(_adjustPending.abs())}',
                          style: TextStyle(
                            color: _adjustPending > 0
                                ? Colors.green
                                : _adjustPending < 0
                                    ? cs.error
                                    : cs.onSurfaceVariant,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        const SizedBox(height: 4),
                        Row(
                          children: [
                            _AdjBtn(
                              label: '−15m',
                              onTap: () => setState(() => _adjustPending -= 15),
                            ),
                            const SizedBox(width: 4),
                            _AdjBtn(
                              label: '+15m',
                              onTap: () => setState(() => _adjustPending += 15),
                            ),
                          ],
                        ),
                        if (_adjustPending != 0) ...[
                          const SizedBox(height: 8),
                          Row(
                            children: [
                              TextButton(
                                onPressed: () => setState(() => _adjustPending = 0),
                                style: TextButton.styleFrom(
                                    padding: const EdgeInsets.symmetric(horizontal: 8)),
                                child: Text(l.reset),
                              ),
                              FilledButton(
                                onPressed: _applyingAdj ? null : _applyAdjustment,
                                style: FilledButton.styleFrom(
                                    padding: const EdgeInsets.symmetric(horizontal: 8)),
                                child: _applyingAdj
                                    ? const SizedBox(
                                        height: 14,
                                        width: 14,
                                        child: CircularProgressIndicator(strokeWidth: 2))
                                    : Text(l.apply),
                              ),
                            ],
                          ),
                        ],
                      ],
                    ),
                  ],
                ),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(
                      child: OutlinedButton.icon(
                        onPressed: _locking ? null : _lockNow,
                        icon: _locking
                            ? const SizedBox(
                                height: 14,
                                width: 14,
                                child: CircularProgressIndicator(strokeWidth: 2))
                            : const Icon(Icons.lock_outlined),
                        label: Text(l.lockNow),
                        style: OutlinedButton.styleFrom(
                          foregroundColor: cs.error,
                          side: BorderSide(color: cs.error),
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: OutlinedButton.icon(
                        onPressed: _showMessageDialog,
                        icon: const Icon(Icons.message_outlined),
                        label: Text(l.message),
                      ),
                    ),
                  ],
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  Widget _buildLanguageCard(BuildContext context, Profile profile) {
    final l = AppLocalizations.of(context);
    const langs = {
      'en': 'English',
      'es': 'Español',
      'fr': 'Français',
      'de': 'Deutsch',
      'pt': 'Português',
      'pl': 'Polski',
    };
    String selected = profile.language;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(l.notifLanguage, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 12),
            StatefulBuilder(builder: (ctx, setState) {
              return Row(
                children: [
                  Expanded(
                    child: DropdownButtonFormField<String>(
                      initialValue: selected,
                      decoration: const InputDecoration(border: OutlineInputBorder()),
                      items: langs.entries
                          .map((e) => DropdownMenuItem(
                              value: e.key, child: Text(e.value)))
                          .toList(),
                      onChanged: (v) => setState(() => selected = v!),
                    ),
                  ),
                  const SizedBox(width: 8),
                  FilledButton(
                    onPressed: () => _saveLanguage(selected),
                    child: Text(l.save),
                  ),
                ],
              );
            }),
          ],
        ),
      ),
    );
  }

  Widget _buildLinkedUsersCard(BuildContext context, Profile profile) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    if (profile.agentUsers.isEmpty) return const SizedBox();
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(l.linkedAccounts, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 8),
            ...profile.agentUsers.map((u) => ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: CircleAvatar(
                    backgroundColor: cs.secondaryContainer,
                    child: Icon(Icons.person, color: cs.onSecondaryContainer),
                  ),
                  title: Text(u.localUsername),
                  subtitle: Text('UID ${u.localUid}'),
                  trailing: Chip(
                    label: Text(u.status),
                    labelStyle: TextStyle(fontSize: 11, color: cs.onPrimaryContainer),
                    backgroundColor: cs.primaryContainer,
                  ),
                )),
          ],
        ),
      ),
    );
  }
}

class _AdjBtn extends StatelessWidget {
  final String label;
  final VoidCallback onTap;
  const _AdjBtn({required this.label, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        decoration: BoxDecoration(
          color: cs.primaryContainer,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Text(label,
            style: TextStyle(color: cs.onPrimaryContainer, fontWeight: FontWeight.w600)),
      ),
    );
  }
}
