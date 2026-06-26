import 'dart:convert';
import 'package:http/http.dart' as http;

class ApiException implements Exception {
  final int statusCode;
  final String message;
  const ApiException(this.statusCode, this.message);
  @override
  String toString() => message;
}

class ApiClient {
  final String baseUrl;
  final String? token;

  ApiClient({required this.baseUrl, this.token});

  String get _base {
    final url = baseUrl.endsWith('/') ? baseUrl.substring(0, baseUrl.length - 1) : baseUrl;
    return '$url/api/v1';
  }

  Map<String, String> get _headers => {
    'Content-Type': 'application/json',
    if (token != null) 'Authorization': 'Bearer $token',
  };

  Future<dynamic> get(String path) async {
    final res = await http
        .get(Uri.parse('$_base$path'), headers: _headers)
        .timeout(const Duration(seconds: 10));
    return _handle(res);
  }

  Future<dynamic> post(String path, [Object? body]) async {
    final res = await http
        .post(Uri.parse('$_base$path'), headers: _headers,
            body: body != null ? jsonEncode(body) : null)
        .timeout(const Duration(seconds: 10));
    return _handle(res);
  }

  Future<dynamic> put(String path, Object body) async {
    final res = await http
        .put(Uri.parse('$_base$path'), headers: _headers, body: jsonEncode(body))
        .timeout(const Duration(seconds: 10));
    return _handle(res);
  }

  Future<dynamic> patch(String path, Object body) async {
    final res = await http
        .patch(Uri.parse('$_base$path'), headers: _headers, body: jsonEncode(body))
        .timeout(const Duration(seconds: 10));
    return _handle(res);
  }

  Future<dynamic> delete(String path) async {
    final res = await http
        .delete(Uri.parse('$_base$path'), headers: _headers)
        .timeout(const Duration(seconds: 10));
    return _handle(res);
  }

  dynamic _handle(http.Response res) {
    if (res.statusCode >= 200 && res.statusCode < 300) {
      if (res.body.isEmpty) return null;
      return jsonDecode(res.body);
    }
    String message = 'Request failed (${res.statusCode})';
    try {
      final body = jsonDecode(res.body) as Map;
      message = (body['error'] as String?) ?? message;
    } catch (_) {}
    if (res.statusCode == 401) throw const UnauthorizedException();
    throw ApiException(res.statusCode, message);
  }
}

class UnauthorizedException extends ApiException {
  const UnauthorizedException() : super(401, 'Session expired. Please sign in again.');
}
