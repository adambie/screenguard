import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../l10n.dart';
import '../widgets/screen_guard_logo.dart';

class LoginScreen extends ConsumerStatefulWidget {
  const LoginScreen({super.key});

  @override
  ConsumerState<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends ConsumerState<LoginScreen> {
  final _userCtrl = TextEditingController();
  final _passCtrl = TextEditingController();
  bool _loading = false;
  bool _setupNeeded = false;
  bool _checkingSetup = true;
  String? _error;

  @override
  void initState() {
    super.initState();
    _checkSetup();
  }

  @override
  void dispose() {
    _userCtrl.dispose();
    _passCtrl.dispose();
    super.dispose();
  }

  Future<void> _checkSetup() async {
    try {
      final data =
          await ref.read(apiClientProvider).get('/auth/status') as Map<String, dynamic>;
      if (mounted) setState(() => _setupNeeded = data['setup_needed'] == true);
    } catch (_) {}
    if (mounted) setState(() => _checkingSetup = false);
  }

  Future<void> _submit() async {
    final username = _userCtrl.text.trim();
    final password = _passCtrl.text;
    if (username.isEmpty || password.isEmpty) return;

    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final client = ref.read(apiClientProvider);
      if (_setupNeeded) {
        await client.post('/auth/setup', {'username': username, 'password': password});
      }
      final data = await client
          .post('/auth/login', {'username': username, 'password': password})
          as Map<String, dynamic>;
      await ref.read(authProvider.notifier).login(username, password, data['token'] as String);
    } on ApiException catch (e) {
      if (mounted) setState(() => _error = e.message);
    } catch (_) {
      if (mounted) {
        setState(() => _error = AppLocalizations.of(context).connectionError);
      }
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_checkingSetup) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }

    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;

    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 48),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const ScreenGuardLogo(size: 72),
              const SizedBox(height: 24),
              Text(
                _setupNeeded ? l.createAdmin : l.signIn,
                style: Theme.of(context)
                    .textTheme
                    .headlineMedium
                    ?.copyWith(fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 8),
              Text(
                _setupNeeded ? l.createAdminDesc : l.signInDesc,
                style: Theme.of(context)
                    .textTheme
                    .bodyMedium
                    ?.copyWith(color: cs.onSurfaceVariant),
              ),
              const SizedBox(height: 32),
              TextField(
                controller: _userCtrl,
                autocorrect: false,
                textCapitalization: TextCapitalization.none,
                textInputAction: TextInputAction.next,
                decoration: InputDecoration(
                  labelText: l.username,
                  border: const OutlineInputBorder(),
                  prefixIcon: const Icon(Icons.person_outline),
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: _passCtrl,
                obscureText: true,
                textInputAction: TextInputAction.done,
                onSubmitted: (_) => _submit(),
                decoration: InputDecoration(
                  labelText: l.password,
                  border: const OutlineInputBorder(),
                  prefixIcon: const Icon(Icons.lock_outline),
                ),
              ),
              if (_error != null) ...[
                const SizedBox(height: 12),
                Text(_error!, style: TextStyle(color: cs.error, fontSize: 13)),
              ],
              const SizedBox(height: 24),
              SizedBox(
                width: double.infinity,
                height: 52,
                child: FilledButton(
                  onPressed: _loading ? null : _submit,
                  child: _loading
                      ? const SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : Text(_setupNeeded ? l.createAccount : l.signIn),
                ),
              ),
              const SizedBox(height: 16),
              Center(
                child: TextButton(
                  onPressed: () => ref.read(authProvider.notifier).clearServerUrl(),
                  child: Text(l.changeServer),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

