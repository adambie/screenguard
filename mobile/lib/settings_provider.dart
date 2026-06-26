import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class ThemeModeNotifier extends StateNotifier<ThemeMode> {
  ThemeModeNotifier() : super(ThemeMode.system) {
    _load();
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    state = ThemeMode.values[(prefs.getInt('theme_mode') ?? 0).clamp(0, 2)];
  }

  Future<void> set(ThemeMode mode) async {
    state = mode;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setInt('theme_mode', mode.index);
  }
}

final themeModeProvider =
    StateNotifierProvider<ThemeModeNotifier, ThemeMode>((ref) => ThemeModeNotifier());

class LanguageNotifier extends StateNotifier<String?> {
  LanguageNotifier() : super(null) {
    _load();
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    state = prefs.getString('app_language');
  }

  Future<void> set(String? code) async {
    state = code;
    final prefs = await SharedPreferences.getInstance();
    if (code == null) {
      await prefs.remove('app_language');
    } else {
      await prefs.setString('app_language', code);
    }
  }
}

final languageProvider =
    StateNotifierProvider<LanguageNotifier, String?>((ref) => LanguageNotifier());
