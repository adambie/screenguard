import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../data_providers.dart';
import '../l10n.dart';
import '../models.dart';
import 'agent_detail_screen.dart';

class AgentsScreen extends ConsumerWidget {
  const AgentsScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final l = AppLocalizations.of(context);
    final agentsAsync = ref.watch(agentsProvider);
    ref.listen(agentsProvider, (_, state) {
      if (state.hasError && state.error is UnauthorizedException) {
        ref.read(authProvider.notifier).relogin();
      }
    });

    return Scaffold(
      appBar: AppBar(
        title: Text(l.devices),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: () => ref.invalidate(agentsProvider),
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: () async => ref.invalidate(agentsProvider),
        child: agentsAsync.when(
          loading: () => const Center(child: CircularProgressIndicator()),
          error: (e, _) => Center(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(l.failedToLoad, style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 8),
                FilledButton(
                  onPressed: () => ref.invalidate(agentsProvider),
                  child: Text(l.retry),
                ),
              ],
            ),
          ),
          data: (agents) {
            if (agents.isEmpty) {
              return Center(
                child: Padding(
                  padding: const EdgeInsets.all(32),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Icon(Icons.computer_outlined, size: 64, color: Colors.grey),
                      const SizedBox(height: 16),
                      Text(
                        l.noDevicesYet,
                        style: const TextStyle(fontSize: 16),
                        textAlign: TextAlign.center,
                      ),
                      const SizedBox(height: 8),
                      Text(
                        l.noDevicesDesc,
                        textAlign: TextAlign.center,
                        style: const TextStyle(color: Colors.grey),
                      ),
                    ],
                  ),
                ),
              );
            }
            return ListView.separated(
              padding: const EdgeInsets.all(16),
              itemCount: agents.length,
              separatorBuilder: (context, index) => const SizedBox(height: 8),
              itemBuilder: (ctx, i) => _AgentCard(agent: agents[i]),
            );
          },
        ),
      ),
    );
  }
}

class _AgentCard extends StatelessWidget {
  final Agent agent;
  const _AgentCard({required this.agent});

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;

    Color statusColor;
    String statusLabel;
    IconData statusIcon;

    switch (agent.status) {
      case 'pending':
        statusColor = cs.tertiary;
        statusLabel = l.statusPending;
        statusIcon = Icons.hourglass_empty;
      case 'pending_delete':
        statusColor = cs.error;
        statusLabel = l.statusRemoving;
        statusIcon = Icons.delete_outline;
      case 'disabled':
        statusColor = cs.onSurfaceVariant;
        statusLabel = l.statusDisabled;
        statusIcon = Icons.block;
      default:
        statusColor = agent.online ? Colors.green : cs.onSurfaceVariant;
        statusLabel = agent.online ? l.statusOnline : l.statusOffline;
        statusIcon = agent.online ? Icons.circle : Icons.circle_outlined;
    }

    return Card(
      child: InkWell(
        borderRadius: BorderRadius.circular(12),
        onTap: () => Navigator.push(
          context,
          MaterialPageRoute(
            builder: (_) => AgentDetailScreen(agentId: agent.id),
          ),
        ),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              CircleAvatar(
                backgroundColor: cs.surfaceContainerHighest,
                child: Icon(Icons.computer, color: cs.onSurfaceVariant),
              ),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      agent.name,
                      style: Theme.of(context)
                          .textTheme
                          .titleMedium
                          ?.copyWith(fontWeight: FontWeight.w600),
                    ),
                    if (agent.displayName.isNotEmpty &&
                        agent.displayName != agent.hostname)
                      Text(
                        agent.hostname,
                        style: TextStyle(color: cs.onSurfaceVariant, fontSize: 13),
                      ),
                    const SizedBox(height: 4),
                    Row(
                      children: [
                        Icon(statusIcon, size: 12, color: statusColor),
                        const SizedBox(width: 4),
                        Text(
                          statusLabel,
                          style: TextStyle(color: statusColor, fontSize: 12),
                        ),
                        if (agent.lastSeenAt != null) ...[
                          Text(' · ',
                              style: TextStyle(color: cs.onSurfaceVariant, fontSize: 12)),
                          Text(
                            l.timeAgo(agent.lastSeenAt),
                            style: TextStyle(color: cs.onSurfaceVariant, fontSize: 12),
                          ),
                        ],
                      ],
                    ),
                  ],
                ),
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  Text(
                    l.userCount(agent.userCount),
                    style: TextStyle(color: cs.onSurfaceVariant, fontSize: 12),
                  ),
                  if (agent.agentVersion != null)
                    Text(
                      agent.agentVersion!,
                      style: TextStyle(color: cs.onSurfaceVariant, fontSize: 11),
                    ),
                  if (agent.upgradeable)
                    Container(
                      margin: const EdgeInsets.only(top: 4),
                      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                      decoration: BoxDecoration(
                        color: Colors.amber.shade100,
                        borderRadius: BorderRadius.circular(8),
                      ),
                      child: Text(
                        l.updateAvailable,
                        style: TextStyle(
                            color: Colors.amber.shade900,
                            fontSize: 10,
                            fontWeight: FontWeight.w600),
                      ),
                    ),
                ],
              ),
              const SizedBox(width: 4),
              Icon(Icons.chevron_right, color: cs.onSurfaceVariant),
            ],
          ),
        ),
      ),
    );
  }
}
