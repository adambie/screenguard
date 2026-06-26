import 'dart:io';
import 'package:flutter/material.dart';
import '../widgets/screen_guard_logo.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:multicast_dns/multicast_dns.dart';
import '../api_client.dart';
import '../auth_provider.dart';
import '../l10n.dart';

class DiscoveredServer {
  final String host;
  final int port;
  final String name;

  const DiscoveredServer({required this.host, required this.port, required this.name});

  String get url => 'http://$host:$port';
}

class ServerSetupScreen extends ConsumerStatefulWidget {
  const ServerSetupScreen({super.key});

  @override
  ConsumerState<ServerSetupScreen> createState() => _ServerSetupScreenState();
}

class _ServerSetupScreenState extends ConsumerState<ServerSetupScreen> {
  final _ctrl = TextEditingController();
  bool _connecting = false;
  bool _scanning = true;
  String? _error;
  final List<DiscoveredServer> _discovered = [];

  @override
  void initState() {
    super.initState();
    _scan();
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  Future<void> _scan() async {
    setState(() {
      _scanning = true;
      _discovered.clear();
    });

    try {
      // reusePort is not supported on GrapheneOS/some Android kernels
      final client = MDnsClient(
        rawDatagramSocketFactory: (dynamic host, int port,
            {bool? reuseAddress, bool? reusePort, int? ttl}) {
          return RawDatagramSocket.bind(
            host, port,
            reuseAddress: reuseAddress ?? true,
            reusePort: false,
            ttl: ttl ?? 1,
          );
        },
      );
      await client.start();

      await for (final PtrResourceRecord ptr in client
          .lookup<PtrResourceRecord>(
              ResourceRecordQuery.serverPointer('_parctrl._tcp.local.'))
          .timeout(const Duration(seconds: 5), onTimeout: (sink) => sink.close())) {
        await for (final SrvResourceRecord srv in client
            .lookup<SrvResourceRecord>(ResourceRecordQuery.service(ptr.domainName))
            .timeout(const Duration(seconds: 3), onTimeout: (sink) => sink.close())) {
          await for (final IPAddressResourceRecord ip in client
              .lookup<IPAddressResourceRecord>(
                  ResourceRecordQuery.addressIPv4(srv.target))
              .timeout(const Duration(seconds: 3), onTimeout: (sink) => sink.close())) {
            final server = DiscoveredServer(
              host: ip.address.address,
              port: srv.port,
              name: ptr.domainName
                  .replaceAll('._parctrl._tcp.local.', '')
                  .replaceAll('pc-server.', ''),
            );
            if (mounted && !_discovered.any((s) => s.url == server.url)) {
              setState(() => _discovered.add(server));
            }
          }
        }
      }

      client.stop();
    } catch (_) {}

    if (mounted) setState(() => _scanning = false);
  }

  Future<void> _connect(String url) async {
    setState(() {
      _connecting = true;
      _error = null;
    });

    try {
      await ApiClient(baseUrl: url).get('/auth/status');
      await ref.read(authProvider.notifier).setServerUrl(url);
    } catch (_) {
      if (mounted) {
        setState(() => _error = AppLocalizations.of(context).cannotReachAt(url));
      }
    } finally {
      if (mounted) setState(() => _connecting = false);
    }
  }

  Future<void> _connectManual() async {
    final input = _ctrl.text.trim();
    if (input.isEmpty) return;

    if (input.startsWith('http://') || input.startsWith('https://')) {
      await _connect(input);
      return;
    }

    setState(() { _connecting = true; _error = null; });
    final httpUrl = 'http://$input';
    final httpsUrl = 'https://$input';
    try {
      await ApiClient(baseUrl: httpUrl).get('/auth/status');
      await ref.read(authProvider.notifier).setServerUrl(httpUrl);
    } catch (_) {
      try {
        await ApiClient(baseUrl: httpsUrl).get('/auth/status');
        await ref.read(authProvider.notifier).setServerUrl(httpsUrl);
      } catch (_) {
        if (mounted) {
          setState(() => _error = AppLocalizations.of(context).cannotReachAddr);
        }
      }
    } finally {
      if (mounted) setState(() => _connecting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final cs = Theme.of(context).colorScheme;

    return Scaffold(
      body: SafeArea(
        child: SingleChildScrollView(
          padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 48),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const ScreenGuardLogo(size: 72),
              const SizedBox(height: 24),
              Text(
                l.connectToServer,
                style: Theme.of(context)
                    .textTheme
                    .headlineMedium
                    ?.copyWith(fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 8),
              Text(
                l.findServerDesc,
                style: Theme.of(context)
                    .textTheme
                    .bodyMedium
                    ?.copyWith(color: cs.onSurfaceVariant),
              ),
              const SizedBox(height: 32),

              Row(
                children: [
                  Text(l.nearbyServers,
                      style: Theme.of(context).textTheme.titleSmall),
                  const Spacer(),
                  if (_scanning)
                    const SizedBox(
                      height: 16,
                      width: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  else
                    TextButton.icon(
                      onPressed: _scan,
                      icon: const Icon(Icons.refresh, size: 16),
                      label: Text(l.scan),
                      style: TextButton.styleFrom(
                          padding: const EdgeInsets.symmetric(horizontal: 8)),
                    ),
                ],
              ),
              const SizedBox(height: 8),

              if (!_scanning && _discovered.isEmpty)
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(16),
                  decoration: BoxDecoration(
                    color: cs.surfaceContainerHighest,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Text(
                    l.noServersFound,
                    style: TextStyle(color: cs.onSurfaceVariant),
                    textAlign: TextAlign.center,
                  ),
                )
              else
                ...(_discovered.map((s) => Padding(
                      padding: const EdgeInsets.only(bottom: 8),
                      child: Card(
                        margin: EdgeInsets.zero,
                        child: ListTile(
                          leading: Icon(Icons.dns, color: cs.primary),
                          title: Text(s.name.isNotEmpty ? s.name : 'ScreenGuard'),
                          subtitle: Text(s.url),
                          trailing: _connecting
                              ? const SizedBox(
                                  height: 20,
                                  width: 20,
                                  child: CircularProgressIndicator(strokeWidth: 2),
                                )
                              : FilledButton(
                                  onPressed: () => _connect(s.url),
                                  child: Text(l.connect),
                                ),
                        ),
                      ),
                    ))),

              const SizedBox(height: 24),
              const Divider(),
              const SizedBox(height: 16),

              Text(l.manualEntry, style: Theme.of(context).textTheme.titleSmall),
              const SizedBox(height: 8),
              TextField(
                controller: _ctrl,
                keyboardType: TextInputType.url,
                autocorrect: false,
                textInputAction: TextInputAction.go,
                onSubmitted: (_) => _connectManual(),
                decoration: InputDecoration(
                  labelText: l.serverAddress,
                  hintText: l.serverAddressHint,
                  border: const OutlineInputBorder(),
                  errorText: _error,
                  prefixIcon: const Icon(Icons.dns_outlined),
                ),
              ),
              const SizedBox(height: 12),
              SizedBox(
                width: double.infinity,
                height: 52,
                child: FilledButton(
                  onPressed: _connecting ? null : _connectManual,
                  child: _connecting
                      ? const SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : Text(l.connectManually),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
