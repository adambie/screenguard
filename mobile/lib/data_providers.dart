import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'auth_provider.dart';
import 'models.dart';

final dashboardProvider =
    FutureProvider.autoDispose<(List<ProfileSummary>, int)>((ref) async {
  final client = ref.watch(apiClientProvider);
  final data = await client.get('/dashboard') as Map<String, dynamic>;
  final profiles = ((data['profiles'] as List?) ?? [])
      .map((p) => ProfileSummary.fromJson(p as Map<String, dynamic>))
      .toList();
  final pending = (data['pending_agents'] as int?) ?? 0;
  return (profiles, pending);
});

final profileDetailProvider =
    FutureProvider.autoDispose.family<Profile, String>((ref, id) async {
  final client = ref.watch(apiClientProvider);
  final data = await client.get('/profiles/$id') as Map<String, dynamic>;
  return Profile.fromJson(data);
});

final profileStatusProvider =
    FutureProvider.autoDispose.family<ProfileStatus, String>((ref, id) async {
  final client = ref.watch(apiClientProvider);
  final data =
      await client.get('/profiles/$id/status') as Map<String, dynamic>;
  return ProfileStatus.fromJson(data);
});

final profilesListProvider =
    FutureProvider.autoDispose<List<ProfileSummary>>((ref) async {
  final client = ref.watch(apiClientProvider);
  final data = await client.get('/profiles') as Map<String, dynamic>;
  return ((data['profiles'] as List?) ?? [])
      .map((p) => ProfileSummary.fromJson(p as Map<String, dynamic>))
      .toList();
});

final agentsProvider =
    FutureProvider.autoDispose<List<Agent>>((ref) async {
  final client = ref.watch(apiClientProvider);
  final data = await client.get('/agents') as Map<String, dynamic>;
  return ((data['agents'] as List?) ?? [])
      .map((a) => Agent.fromJson(a as Map<String, dynamic>))
      .toList();
});

final agentDetailProvider =
    FutureProvider.autoDispose.family<Agent, String>((ref, id) async {
  final client = ref.watch(apiClientProvider);
  final data = await client.get('/agents/$id') as Map<String, dynamic>;
  return Agent.fromJson(data);
});

final agentUsersProvider =
    FutureProvider.autoDispose.family<List<AgentUser>, String>((ref, id) async {
  final client = ref.watch(apiClientProvider);
  final data =
      await client.get('/agents/$id/users') as Map<String, dynamic>;
  return ((data['users'] as List?) ?? [])
      .map((u) => AgentUser.fromJson(u as Map<String, dynamic>))
      .toList();
});
