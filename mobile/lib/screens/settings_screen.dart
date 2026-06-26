import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../auth_provider.dart';
import '../l10n.dart';
import '../settings_provider.dart';

class SettingsScreen extends ConsumerWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
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
                DropdownMenuItem(value: ThemeMode.system, child: Text(l.themeSystem)),
                DropdownMenuItem(value: ThemeMode.light, child: Text(l.themeLight)),
                DropdownMenuItem(value: ThemeMode.dark, child: Text(l.themeDark)),
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
