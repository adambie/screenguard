import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../l10n.dart';
import '../settings_provider.dart';

const _commonTimezones = [
  'UTC',
  'Africa/Cairo', 'Africa/Johannesburg', 'Africa/Lagos', 'Africa/Nairobi',
  'America/Anchorage', 'America/Argentina/Buenos_Aires', 'America/Bogota',
  'America/Chicago', 'America/Denver', 'America/Los_Angeles', 'America/Mexico_City',
  'America/New_York', 'America/Phoenix', 'America/Sao_Paulo', 'America/Toronto',
  'America/Vancouver',
  'Asia/Bangkok', 'Asia/Colombo', 'Asia/Dubai', 'Asia/Hong_Kong', 'Asia/Jakarta',
  'Asia/Karachi', 'Asia/Kolkata', 'Asia/Kuala_Lumpur', 'Asia/Seoul',
  'Asia/Shanghai', 'Asia/Singapore', 'Asia/Taipei', 'Asia/Tokyo',
  'Atlantic/Reykjavik',
  'Australia/Adelaide', 'Australia/Brisbane', 'Australia/Melbourne',
  'Australia/Perth', 'Australia/Sydney',
  'Europe/Amsterdam', 'Europe/Athens', 'Europe/Belgrade', 'Europe/Berlin',
  'Europe/Brussels', 'Europe/Bucharest', 'Europe/Budapest', 'Europe/Copenhagen',
  'Europe/Dublin', 'Europe/Helsinki', 'Europe/Istanbul', 'Europe/Kiev',
  'Europe/Lisbon', 'Europe/London', 'Europe/Madrid', 'Europe/Moscow',
  'Europe/Oslo', 'Europe/Paris', 'Europe/Prague', 'Europe/Rome',
  'Europe/Sofia', 'Europe/Stockholm', 'Europe/Vienna', 'Europe/Warsaw',
  'Europe/Zurich',
  'Pacific/Auckland', 'Pacific/Fiji', 'Pacific/Honolulu',
];

class SettingsScreen extends ConsumerStatefulWidget {
  const SettingsScreen({super.key});

  @override
  ConsumerState<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends ConsumerState<SettingsScreen> {
  String? _adminTimezone;
  bool _tzLoading = false;

  @override
  void initState() {
    super.initState();
    _loadTimezone();
  }

  Future<void> _loadTimezone() async {
    try {
      final data = await ref.read(apiClientProvider).get('/auth/me');
      if (mounted) setState(() => _adminTimezone = data['timezone'] as String?);
    } catch (_) {}
  }

  Future<void> _pickTimezone() async {
    final l = AppLocalizations.of(context);
    final filter = TextEditingController();
    String? picked;
    await showDialog<void>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setS) {
          final filtered = filter.text.isEmpty
              ? _commonTimezones
              : _commonTimezones
                  .where((z) => z.toLowerCase().contains(filter.text.toLowerCase()))
                  .toList();
          return AlertDialog(
            title: Text(l.selectTimezone),
            content: SizedBox(
              width: 300,
              height: 400,
              child: Column(
                children: [
                  TextField(
                    controller: filter,
                    decoration: InputDecoration(
                      hintText: l.search,
                      prefixIcon: const Icon(Icons.search),
                      border: const OutlineInputBorder(),
                      isDense: true,
                    ),
                    onChanged: (_) => setS(() {}),
                  ),
                  const SizedBox(height: 8),
                  Expanded(
                    child: ListView.builder(
                      itemCount: filtered.length,
                      itemBuilder: (_, i) => ListTile(
                        title: Text(filtered[i],
                            style: const TextStyle(fontSize: 13)),
                        selected: filtered[i] == _adminTimezone,
                        onTap: () {
                          picked = filtered[i];
                          Navigator.pop(ctx);
                        },
                      ),
                    ),
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx),
                child: Text(l.cancel),
              ),
            ],
          );
        },
      ),
    );
    if (picked != null && picked != _adminTimezone) {
      setState(() {
        _tzLoading = true;
        _adminTimezone = picked;
      });
      try {
        await ref.read(apiClientProvider).patch('/auth/me', {'timezone': picked});
      } on UnauthorizedException {
        ref.read(authProvider.notifier).relogin();
      } on ApiException catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(e.message),
              backgroundColor: Theme.of(context).colorScheme.error,
            ),
          );
        }
      } finally {
        if (mounted) setState(() => _tzLoading = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final themeMode = ref.watch(themeModeProvider);
    final language = ref.watch(languageProvider);

    final langOptions = [
      (code: null as String?, name: l.languageAuto),
      (code: 'en', name: 'English'),
      (code: 'es', name: 'Español'),
      (code: 'fr', name: 'Français'),
      (code: 'de', name: 'Deutsch'),
      (code: 'pt', name: 'Português'),
      (code: 'pl', name: 'Polski'),
    ];

    return Scaffold(
      appBar: AppBar(title: Text(l.settingsTitle)),
      body: ListView(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 4),
            child: Text(
              l.appearance,
              style: Theme.of(context)
                  .textTheme
                  .labelLarge
                  ?.copyWith(color: Theme.of(context).colorScheme.primary),
            ),
          ),
          ListTile(
            title: Text(l.theme),
            trailing: DropdownButton<ThemeMode>(
              value: themeMode,
              underline: const SizedBox(),
              items: [
                DropdownMenuItem(
                    value: ThemeMode.system, child: Text(l.themeSystem)),
                DropdownMenuItem(
                    value: ThemeMode.light, child: Text(l.themeLight)),
                DropdownMenuItem(
                    value: ThemeMode.dark, child: Text(l.themeDark)),
              ],
              onChanged: (v) {
                if (v != null) ref.read(themeModeProvider.notifier).set(v);
              },
            ),
          ),
          ListTile(
            title: Text(l.appLanguage),
            trailing: DropdownButton<String?>(
              value: language,
              underline: const SizedBox(),
              items: langOptions
                  .map((e) => DropdownMenuItem<String?>(
                        value: e.code,
                        child: Text(e.name),
                      ))
                  .toList(),
              onChanged: (v) => ref.read(languageProvider.notifier).set(v),
            ),
          ),
          const Divider(),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: Text(
              l.account,
              style: Theme.of(context)
                  .textTheme
                  .labelLarge
                  ?.copyWith(color: Theme.of(context).colorScheme.primary),
            ),
          ),
          ListTile(
            leading: const Icon(Icons.schedule),
            title: Text(l.adminTimezone),
            subtitle: Text(_adminTimezone ?? l.notSet,
                style: const TextStyle(fontSize: 12)),
            trailing: _tzLoading
                ? const SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(strokeWidth: 2))
                : const Icon(Icons.chevron_right),
            onTap: _tzLoading ? null : _pickTimezone,
          ),
          ListTile(
            leading: const Icon(Icons.logout),
            title: Text(l.signOut),
            onTap: () => ref.read(authProvider.notifier).logout(),
          ),
          ListTile(
            leading: const Icon(Icons.dns_outlined),
            title: Text(l.changeServer),
            onTap: () => ref.read(authProvider.notifier).clearServerUrl(),
          ),
        ],
      ),
    );
  }
}
