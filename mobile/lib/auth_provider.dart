import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'api_client.dart';

class AuthState {
  final String? serverUrl;
  final String? token;
  final bool isLoading;

  const AuthState({this.serverUrl, this.token, this.isLoading = false});

  bool get isLoggedIn => token != null;

  AuthState _copy({Object? serverUrl = _s, Object? token = _s, bool? isLoading}) => AuthState(
        serverUrl: serverUrl == _s ? this.serverUrl : serverUrl as String?,
        token: token == _s ? this.token : token as String?,
        isLoading: isLoading ?? this.isLoading,
      );
}

const _s = Object();

class AuthNotifier extends StateNotifier<AuthState> {
  AuthNotifier() : super(const AuthState(isLoading: true)) {
    _init();
  }

  static const _storage = FlutterSecureStorage();
  bool _relogging = false;

  Future<void> _init() async {
    final prefs = await SharedPreferences.getInstance();
    final serverUrl = prefs.getString('server_url');
    final token = await _storage.read(key: 'auth_token');
    state = AuthState(serverUrl: serverUrl, token: token);
  }

  Future<void> setServerUrl(String url) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('server_url', url);
    state = state._copy(serverUrl: url, token: null);
  }

  Future<void> clearServerUrl() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove('server_url');
    await _storage.deleteAll();
    state = const AuthState();
  }

  Future<void> login(String username, String password, String token) async {
    await _storage.write(key: 'auth_token', value: token);
    await _storage.write(key: 'auth_username', value: username);
    await _storage.write(key: 'auth_password', value: password);
    state = state._copy(token: token);
  }

  Future<void> relogin() async {
    if (_relogging) return;
    _relogging = true;
    try {
      final username = await _storage.read(key: 'auth_username');
      final password = await _storage.read(key: 'auth_password');
      if (username == null || password == null) {
        await logout();
        return;
      }
      final client = ApiClient(baseUrl: state.serverUrl ?? 'http://localhost:8080');
      final data = await client.post('/auth/login', {'username': username, 'password': password})
          as Map<String, dynamic>;
      final token = data['token'] as String;
      await _storage.write(key: 'auth_token', value: token);
      state = state._copy(token: token);
    } catch (_) {
      await logout();
    } finally {
      _relogging = false;
    }
  }

  Future<void> logout() async {
    await _storage.deleteAll();
    state = AuthState(serverUrl: state.serverUrl);
  }
}

final authProvider = StateNotifierProvider<AuthNotifier, AuthState>(
  (ref) => AuthNotifier(),
);

final apiClientProvider = Provider<ApiClient>((ref) {
  final auth = ref.watch(authProvider);
  return ApiClient(
    baseUrl: auth.serverUrl ?? 'http://localhost:8080',
    token: auth.token,
  );
});
