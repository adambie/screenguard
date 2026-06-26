import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../data_providers.dart';
import '../l10n.dart';
import '../models.dart';
import '../utils.dart';
import 'profile_detail_screen.dart';

class DashboardScreen extends ConsumerStatefulWidget {
  const DashboardScreen({super.key});

  @override
  ConsumerState<DashboardScreen> createState() => _DashboardScreenState();
}

class _DashboardScreenState extends ConsumerState<DashboardScreen> {
  Future<void> _createProfile(String name) async {
    try {
      await ref.read(apiClientProvider).post('/profiles', {'display_name': name});
      ref.invalidate(dashboardProvider);
    } on UnauthorizedException {
      ref.read(authProvider.notifier).relogin();
    } catch (e) {
      if (mounted) _showError(e.toString());
    }
  }

  void _showCreateDialog() {
    final l = AppLocalizations.of(context);
    final ctrl = TextEditingController();
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(l.newProfile),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          textCapitalization: TextCapitalization.words,
          decoration: InputDecoration(
            labelText: l.profileNameLabel,
            border: const OutlineInputBorder(),
          ),
          onSubmitted: (v) {
            if (v.trim().isNotEmpty) {
              Navigator.pop(ctx);
              _createProfile(v.trim());
            }
          },
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: Text(l.cancel)),
          FilledButton(
            onPressed: () {
              if (ctrl.text.trim().isNotEmpty) {
                Navigator.pop(ctx);
                _createProfile(ctrl.text.trim());
              }
            },
            child: Text(l.create),
          ),
        ],
      ),
    );
  }

  void _showError(String msg) {
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(msg), backgroundColor: Colors.red));
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final dashAsync = ref.watch(dashboardProvider);
    ref.listen(dashboardProvider, (_, state) {
      if (state.hasError && state.error is UnauthorizedException) {
        ref.read(authProvider.notifier).relogin();
      }
    });

    return Scaffold(
      appBar: AppBar(
        title: const Text('ScreenGuard'),
        actions: [
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert),
            onSelected: (v) {
              if (v == 'signout') ref.read(authProvider.notifier).logout();
            },
            itemBuilder: (_) => [
              PopupMenuItem(value: 'signout', child: Text(l.signOut)),
            ],
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: () async => ref.invalidate(dashboardProvider),
        child: dashAsync.when(
          loading: () => const Center(child: CircularProgressIndicator()),
          error: (e, _) => Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(l.failedToLoad, style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 8),
                Text(e.toString(),
                    style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant)),
                const SizedBox(height: 16),
                FilledButton(
                  onPressed: () => ref.invalidate(dashboardProvider),
                  child: Text(l.retry),
                ),
              ],
            ),
          ),
          data: (data) {
            final (profiles, pendingAgents) = data;
            return CustomScrollView(
              slivers: [
                if (pendingAgents > 0)
                  SliverToBoxAdapter(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(16, 16, 16, 0),
                      child: _PendingAgentsBanner(count: pendingAgents),
                    ),
                  ),
                if (profiles.isEmpty)
                  SliverFillRemaining(
                    child: Center(child: Text(l.noProfilesYet)),
                  )
                else
                  SliverPadding(
                    padding: const EdgeInsets.all(16),
                    sliver: SliverList.separated(
                      itemCount: profiles.length,
                      separatorBuilder: (context, index) => const SizedBox(height: 12),
                      itemBuilder: (ctx, i) => _ProfileCard(profile: profiles[i]),
                    ),
                  ),
              ],
            );
          },
        ),
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _showCreateDialog,
        icon: const Icon(Icons.add),
        label: Text(l.newProfile),
      ),
    );
  }
}

class _PendingAgentsBanner extends StatelessWidget {
  final int count;
  const _PendingAgentsBanner({required this.count});

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    return Card(
      color: cs.tertiaryContainer,
      child: ListTile(
        leading: Icon(Icons.computer, color: cs.tertiary),
        title: Text(
          l.devicesWaiting(count),
          style: TextStyle(color: cs.onTertiaryContainer),
        ),
        trailing: Icon(Icons.chevron_right, color: cs.tertiary),
      ),
    );
  }
}

class _ProfileCard extends StatelessWidget {
  final ProfileSummary profile;
  const _ProfileCard({required this.profile});

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final color = profileColor(profile.displayName);
    final isLocked = profile.enforce == 'lock';

    double? progress;
    if (profile.limitMinutes != null && profile.limitMinutes! > 0) {
      progress = (profile.usedMinutes / profile.limitMinutes!).clamp(0.0, 1.0);
    }

    return Card(
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: () => Navigator.push(
          context,
          MaterialPageRoute(
            builder: (_) => ProfileDetailScreen(profileId: profile.id),
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 6, color: isLocked ? cs.error : color),
            Padding(
              padding: const EdgeInsets.all(16),
              child: Row(
                children: [
                  CircleAvatar(
                    backgroundColor: color,
                    foregroundColor: Colors.white,
                    radius: 24,
                    child: Text(
                      profile.displayName.isNotEmpty
                          ? profile.displayName[0].toUpperCase()
                          : '?',
                      style: const TextStyle(fontSize: 20, fontWeight: FontWeight.bold),
                    ),
                  ),
                  const SizedBox(width: 16),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            Expanded(
                              child: Text(
                                profile.displayName,
                                style: Theme.of(context)
                                    .textTheme
                                    .titleMedium
                                    ?.copyWith(fontWeight: FontWeight.w600),
                              ),
                            ),
                            if (isLocked)
                              Chip(
                                label: Text(l.locked),
                                labelStyle: TextStyle(color: cs.onError, fontSize: 11),
                                backgroundColor: cs.error,
                                padding: EdgeInsets.zero,
                                materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                              ),
                          ],
                        ),
                        const SizedBox(height: 4),
                        _buildTimeText(context),
                      ],
                    ),
                  ),
                ],
              ),
            ),
            if (progress != null)
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 0, 16, 4),
                child: LinearProgressIndicator(
                  value: progress,
                  backgroundColor: cs.surfaceContainerHighest,
                  color: isLocked
                      ? cs.error
                      : progress > 0.9
                          ? Colors.orange
                          : color,
                  minHeight: 6,
                  borderRadius: BorderRadius.circular(3),
                ),
              ),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
              child: Row(
                children: [
                  Icon(
                    profile.agentsOnline > 0 ? Icons.computer : Icons.computer_outlined,
                    size: 14,
                    color: cs.onSurfaceVariant,
                  ),
                  const SizedBox(width: 4),
                  Text(
                    l.devicesOnline(profile.agentsOnline, profile.agentsTotal),
                    style: Theme.of(context)
                        .textTheme
                        .bodySmall
                        ?.copyWith(color: cs.onSurfaceVariant),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildTimeText(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final style =
        Theme.of(context).textTheme.bodySmall?.copyWith(color: cs.onSurfaceVariant);

    if (profile.limitMinutes == null) {
      return Text(l.usedNoLimit(formatMinutes(profile.usedMinutes)), style: style);
    }

    final remaining = profile.remainingMinutes ?? 0;
    return Text(
      l.usedWithLimit(
        formatMinutes(profile.usedMinutes),
        formatMinutes(remaining),
        formatMinutes(profile.limitMinutes!),
      ),
      style: style,
    );
  }
}
