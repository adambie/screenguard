import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../data_providers.dart';
import '../l10n.dart';
import '../models.dart';
import '../settings_provider.dart';

class AgentDetailScreen extends ConsumerStatefulWidget {
  final String agentId;
  const AgentDetailScreen({super.key, required this.agentId});

  @override
  ConsumerState<AgentDetailScreen> createState() => _AgentDetailScreenState();
}

class _AgentDetailScreenState extends ConsumerState<AgentDetailScreen> {
  List<String>? _logLines;
  bool _logsLoading = false;
  String? _logsError;

  Future<void> _fetchLogs() async {
    setState(() { _logsLoading = true; _logsError = null; });
    try {
      final data = await ref.read(apiClientProvider).get('/agents/${widget.agentId}/logs');
      setState(() {
        _logLines = List<String>.from(data['lines'] ?? []);
        _logsLoading = false;
      });
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      setState(() { _logsError = e.message; _logsLoading = false; });
    } catch (e) {
      setState(() { _logsError = e.toString(); _logsLoading = false; });
    }
  }

  void _refresh() {
    ref.invalidate(agentDetailProvider(widget.agentId));
    ref.invalidate(agentUsersProvider(widget.agentId));
    ref.invalidate(agentsProvider);
  }

  Future<void> _accept() async {
    try {
      await ref.read(apiClientProvider).post('/agents/${widget.agentId}/accept');
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).deviceAccepted);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _delete() async {
    final l = AppLocalizations.of(context);
    final ok = await _confirm(l.deleteDeviceTitle, l.deleteDeviceBody);
    if (!ok) return;
    try {
      await ref.read(apiClientProvider).delete('/agents/${widget.agentId}');
      _refresh();
      if (mounted) {
        ref.invalidate(agentsProvider);
        Navigator.pop(context);
      }
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _undoDelete() async {
    try {
      await ref
          .read(apiClientProvider)
          .post('/agents/${widget.agentId}/undo-delete');
      _refresh();
      if (mounted) _snack(AppLocalizations.of(context).deletionCancelled);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _forceDelete() async {
    final l = AppLocalizations.of(context);
    final ok = await _confirm(l.forceRemoveTitle, l.forceRemoveBody);
    if (!ok) return;
    try {
      await ref
          .read(apiClientProvider)
          .post('/agents/${widget.agentId}/force-delete');
      if (mounted) {
        ref.invalidate(agentsProvider);
        Navigator.pop(context);
      }
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
  }

  Future<void> _rename(String currentName) async {
    final l = AppLocalizations.of(context);
    final ctrl = TextEditingController(text: currentName);
    final result = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.renameDevice),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          decoration: const InputDecoration(border: OutlineInputBorder()),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: Text(l.cancel)),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, ctrl.text.trim()),
            child: Text(l.save),
          ),
        ],
      ),
    );
    if (result != null && result.isNotEmpty) {
      try {
        await ref.read(apiClientProvider).patch(
          '/agents/${widget.agentId}',
          {'display_name': result},
        );
        _refresh();
      } on UnauthorizedException {
        ref.read(authProvider.notifier).relogin();
      } on ApiException catch (e) {
        if (mounted) _snack(e.message, error: true);
      }
    }
  }

  Future<void> _linkUser(AgentUser user, String? profileId) async {
    try {
      await ref.read(apiClientProvider).patch(
        '/agent-users/${user.id}',
        {
          'profile_id': profileId,
          'status': profileId != null ? 'managed' : 'unmanaged',
        },
      );
      ref.invalidate(agentUsersProvider(widget.agentId));
      if (mounted) {
        final l = AppLocalizations.of(context);
        _snack(profileId != null ? l.userLinked : l.userUnlinked);
      }
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } on ApiException catch (e) {
      if (mounted) _snack(e.message, error: true);
    }
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
    final agentAsync = ref.watch(agentDetailProvider(widget.agentId));
    final usersAsync = ref.watch(agentUsersProvider(widget.agentId));
    final profilesAsync = ref.watch(profilesListProvider);
    ref.listen(agentDetailProvider(widget.agentId), (_, state) {
      if (state.hasError && state.error is UnauthorizedException) {
        ref.read(authProvider.notifier).relogin();
      }
    });

    return Scaffold(
      appBar: AppBar(
        title: agentAsync.when(
          data: (a) => Text(a.name),
          loading: () => Text(l.deviceTitle),
          error: (err, st) => Text(l.deviceTitle),
        ),
        actions: [
          agentAsync.when(
            data: (agent) => PopupMenuButton<String>(
              onSelected: (v) {
                if (v == 'rename') _rename(agent.name);
                if (v == 'delete') _delete();
                if (v == 'undo') _undoDelete();
                if (v == 'force') _forceDelete();
              },
              itemBuilder: (_) => [
                PopupMenuItem(value: 'rename', child: Text(l.rename)),
                if (agent.status == 'pending_delete')
                  PopupMenuItem(value: 'undo', child: Text(l.undoDeletion)),
                if (agent.status == 'pending_delete')
                  PopupMenuItem(value: 'force', child: Text(l.forceRemoveMenu)),
                if (agent.status != 'pending_delete')
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
          child: agentAsync.when(
            loading: () => const Center(child: CircularProgressIndicator()),
            error: (e, _) => Center(child: Text('Error: $e')),
            data: (agent) => Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                _buildInfoCard(context, agent),
                const SizedBox(height: 16),
                if (agent.status == 'pending') ...[
                  _buildPendingCard(context, agent),
                  const SizedBox(height: 16),
                ],
                _buildUsersCard(context, usersAsync, profilesAsync),
                if (ref.watch(showLogsProvider)) ...[
                  const SizedBox(height: 16),
                  _buildLogsCard(context, agent),
                ],
                const SizedBox(height: 32),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _buildInfoCard(BuildContext context, Agent agent) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final isOnline = agent.online;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                CircleAvatar(
                  radius: 28,
                  backgroundColor:
                      isOnline ? cs.primaryContainer : cs.surfaceContainerHighest,
                  child: Icon(
                    Icons.computer,
                    color: isOnline ? cs.onPrimaryContainer : cs.onSurfaceVariant,
                    size: 28,
                  ),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(agent.name,
                          style: Theme.of(context)
                              .textTheme
                              .titleLarge
                              ?.copyWith(fontWeight: FontWeight.bold)),
                      if (agent.displayName.isNotEmpty &&
                          agent.displayName != agent.hostname)
                        Text(agent.hostname,
                            style: TextStyle(color: cs.onSurfaceVariant)),
                      const SizedBox(height: 4),
                      _StatusChip(status: agent.status, online: agent.online),
                    ],
                  ),
                ),
              ],
            ),
            const Divider(height: 24),
            _InfoRow(l.timezone, agent.timezone),
            if (agent.agentVersion != null)
              _InfoRow(l.agentVersion, agent.agentVersion!),
            _InfoRow(l.lastSeen, agent.online ? l.now : l.timeAgo(agent.lastSeenAt)),
            _InfoRow(l.machineId, agent.machineId),
            if (agent.machineId.isNotEmpty)
              Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  onPressed: () {
                    Clipboard.setData(ClipboardData(text: agent.machineId));
                    ScaffoldMessenger.of(context).showSnackBar(
                      SnackBar(content: Text(l.copiedToClipboard)),
                    );
                  },
                  icon: const Icon(Icons.copy, size: 14),
                  label: Text(l.copyId, style: const TextStyle(fontSize: 12)),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _buildPendingCard(BuildContext context, Agent agent) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      color: cs.tertiaryContainer,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.hourglass_empty, color: cs.tertiary),
                const SizedBox(width: 8),
                Text(l.waitingApproval,
                    style: TextStyle(
                        color: cs.onTertiaryContainer,
                        fontWeight: FontWeight.bold)),
              ],
            ),
            const SizedBox(height: 12),
            if (agent.pairingCode != null) ...[
              Text(l.pairingCode,
                  style: TextStyle(color: cs.onTertiaryContainer, fontSize: 12)),
              const SizedBox(height: 4),
              SelectableText(
                agent.pairingCode!,
                style: TextStyle(
                  fontSize: 28,
                  fontFamily: 'monospace',
                  fontWeight: FontWeight.bold,
                  letterSpacing: 6,
                  color: cs.onTertiaryContainer,
                ),
              ),
              const SizedBox(height: 12),
            ],
            SizedBox(
              width: double.infinity,
              child: FilledButton.icon(
                onPressed: _accept,
                icon: const Icon(Icons.check),
                label: Text(l.acceptDevice),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildUsersCard(
    BuildContext context,
    AsyncValue<List<AgentUser>> usersAsync,
    AsyncValue<List<ProfileSummary>> profilesAsync,
  ) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(l.managedUsers, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 12),
            usersAsync.when(
              loading: () => const Center(child: CircularProgressIndicator()),
              error: (e, _) => Text('Error: $e'),
              data: (users) {
                if (users.isEmpty) {
                  return Text(
                    l.noUsersYet,
                    style: TextStyle(color: cs.onSurfaceVariant),
                  );
                }
                final profiles = profilesAsync.valueOrNull ?? [];
                return Column(
                  children: users
                      .map((u) => _UserRow(
                            user: u,
                            profiles: profiles,
                            onLink: (profileId) => _linkUser(u, profileId),
                          ))
                      .toList(),
                );
              },
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildLogsCard(BuildContext context, Agent agent) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(l.recentLogs, style: Theme.of(context).textTheme.titleMedium),
                if (agent.online)
                  FilledButton.tonal(
                    onPressed: _logsLoading ? null : _fetchLogs,
                    child: Text(_logLines == null ? l.loadLogs : l.refreshLogs),
                  ),
              ],
            ),
            if (!agent.online) ...[
              const SizedBox(height: 8),
              Text(l.agentOfflineLogs, style: TextStyle(color: cs.onSurfaceVariant, fontSize: 13)),
            ] else if (_logsLoading) ...[
              const SizedBox(height: 16),
              const Center(child: CircularProgressIndicator()),
            ] else if (_logsError != null) ...[
              const SizedBox(height: 8),
              Text(_logsError!, style: TextStyle(color: cs.error)),
            ] else if (_logLines != null) ...[
              const SizedBox(height: 12),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: cs.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(8),
                ),
                constraints: const BoxConstraints(maxHeight: 360),
                child: _logLines!.isEmpty
                    ? Text(l.logsEmpty, style: TextStyle(color: cs.onSurfaceVariant))
                    : SingleChildScrollView(
                        child: SelectableText(
                          _logLines!.join('\n'),
                          style: const TextStyle(fontSize: 11, fontFamily: 'monospace'),
                        ),
                      ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _UserRow extends StatefulWidget {
  final AgentUser user;
  final List<ProfileSummary> profiles;
  final Future<void> Function(String? profileId) onLink;

  const _UserRow({
    required this.user,
    required this.profiles,
    required this.onLink,
  });

  @override
  State<_UserRow> createState() => _UserRowState();
}

class _UserRowState extends State<_UserRow> {
  String? _selected;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _selected = widget.user.profileId;
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        children: [
          CircleAvatar(
            backgroundColor: cs.surfaceContainerHighest,
            child: Icon(Icons.person, color: cs.onSurfaceVariant),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(widget.user.localUsername,
                    style: const TextStyle(fontWeight: FontWeight.w600)),
                Text('UID ${widget.user.localUid}',
                    style: TextStyle(color: cs.onSurfaceVariant, fontSize: 12)),
              ],
            ),
          ),
          const SizedBox(width: 8),
          DropdownButton<String?>(
            value: _selected,
            underline: const SizedBox(),
            hint: Text(l.unmanaged),
            items: [
              DropdownMenuItem<String?>(value: null, child: Text(l.unmanaged)),
              ...widget.profiles.map((p) =>
                  DropdownMenuItem<String?>(value: p.id, child: Text(p.displayName))),
            ],
            onChanged: _saving
                ? null
                : (v) async {
                    setState(() {
                      _selected = v;
                      _saving = true;
                    });
                    await widget.onLink(v);
                    if (mounted) setState(() => _saving = false);
                  },
          ),
        ],
      ),
    );
  }
}

class _StatusChip extends StatelessWidget {
  final String status;
  final bool online;
  const _StatusChip({required this.status, required this.online});

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    Color bg;
    Color fg;
    String label;

    switch (status) {
      case 'pending':
        bg = cs.tertiaryContainer;
        fg = cs.onTertiaryContainer;
        label = l.statusPendingApproval;
      case 'pending_delete':
        bg = cs.errorContainer;
        fg = cs.onErrorContainer;
        label = l.statusPendingRemoval;
      case 'disabled':
        bg = cs.surfaceContainerHighest;
        fg = cs.onSurfaceVariant;
        label = l.statusDisabled;
      default:
        bg = online ? Colors.green.shade100 : cs.surfaceContainerHighest;
        fg = online ? Colors.green.shade800 : cs.onSurfaceVariant;
        label = online ? l.statusOnline : l.statusOffline;
    }

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
      decoration: BoxDecoration(color: bg, borderRadius: BorderRadius.circular(12)),
      child: Text(label, style: TextStyle(color: fg, fontSize: 12)),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  const _InfoRow(this.label, this.value);

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 100,
            child: Text(label, style: TextStyle(color: cs.onSurfaceVariant)),
          ),
          Expanded(child: Text(value, style: const TextStyle(fontWeight: FontWeight.w500))),
        ],
      ),
    );
  }
}
