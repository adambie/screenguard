import 'package:flutter/material.dart';

String formatMinutes(int minutes) {
  if (minutes <= 0) return '0m';
  final h = minutes ~/ 60;
  final m = minutes % 60;
  if (h == 0) return '${m}m';
  if (m == 0) return '${h}h';
  return '${h}h ${m}m';
}

String formatTimeAgo(int? unixSeconds) {
  if (unixSeconds == null) return 'never';
  final dt = DateTime.fromMillisecondsSinceEpoch(unixSeconds * 1000);
  final diff = DateTime.now().difference(dt);
  if (diff.inSeconds < 60) return 'just now';
  if (diff.inMinutes < 60) return '${diff.inMinutes}m ago';
  if (diff.inHours < 24) return '${diff.inHours}h ago';
  if (diff.inDays < 7) return '${diff.inDays}d ago';
  return '${dt.day}/${dt.month}';
}

String timeOfDayToString(TimeOfDay t) =>
    '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';

TimeOfDay stringToTimeOfDay(String s) {
  final parts = s.split(':');
  return TimeOfDay(hour: int.parse(parts[0]), minute: int.parse(parts[1]));
}

const _dayNames = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const _dayNamesFull = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

String dayName(int dow) => _dayNames[dow.clamp(0, 6)];
String dayNameFull(int dow) => _dayNamesFull[dow.clamp(0, 6)];

Color profileColor(String name) {
  const colors = [
    Color(0xFF4285F4),
    Color(0xFF34A853),
    Color(0xFFEA4335),
    Color(0xFF9C27B0),
    Color(0xFF00897B),
    Color(0xFFFF7043),
    Color(0xFF1E88E5),
    Color(0xFF607D8B),
  ];
  return colors[name.hashCode.abs() % colors.length];
}
