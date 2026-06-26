import 'package:flutter/material.dart';
import '../l10n.dart';
import '../widgets/screen_guard_logo.dart';

const _appVersion = String.fromEnvironment('APP_VERSION', defaultValue: 'dev');
const _repoUrl = 'https://github.com/adambie/screenguard';
const _authorName = 'Adam Bielawny';
const _authorEmail = 'adam.bielawny@protonmail.com';

class AboutScreen extends StatelessWidget {
  const AboutScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;
    final tt = Theme.of(context).textTheme;

    return Scaffold(
      appBar: AppBar(title: Text(l.aboutTitle)),
      body: ListView(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
        children: [
          Center(child: const ScreenGuardLogo(size: 88)),
          const SizedBox(height: 20),
          Center(
            child: Text(
              'ScreenGuard',
              style: tt.headlineSmall?.copyWith(fontWeight: FontWeight.bold),
            ),
          ),
          const SizedBox(height: 4),
          Center(
            child: Text(
              '${l.version}: $_appVersion',
              style: tt.bodyMedium?.copyWith(color: cs.onSurfaceVariant),
            ),
          ),
          const SizedBox(height: 24),
          Text(
            l.appDescription,
            style: tt.bodyMedium,
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 32),
          _InfoTile(label: l.author, value: '$_authorName · $_authorEmail'),
          const Divider(height: 1),
          _InfoTile(label: l.version, value: _appVersion),
          const Divider(height: 1),
          ListTile(
            contentPadding: EdgeInsets.zero,
            title: Text(l.sourceCode),
            subtitle: Text(_repoUrl, style: TextStyle(color: cs.primary)),
          ),
          const SizedBox(height: 16),
          OutlinedButton.icon(
            onPressed: () => showLicensePage(
              context: context,
              applicationName: 'ScreenGuard',
              applicationVersion: _appVersion,
            ),
            icon: const Icon(Icons.description_outlined),
            label: Text(l.licenses),
          ),
        ],
      ),
    );
  }
}

class _InfoTile extends StatelessWidget {
  final String label;
  final String value;
  const _InfoTile({required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      title: Text(label),
      trailing: Text(value, style: Theme.of(context).textTheme.bodyMedium),
    );
  }
}
