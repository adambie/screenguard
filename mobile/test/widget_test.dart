import 'package:flutter_test/flutter_test.dart';
import 'package:screenguard/models.dart';

void main() {
  test('profile defaults preserve tasks on lock to false when absent', () {
    final profile = Profile.fromJson({
      'profile': {'id': 'profile-id', 'display_name': 'Test'},
      'schedules': [],
      'daily_limits': [],
      'agent_users': [],
    });

    expect(profile.preserveTasksOnLock, isFalse);
  });

  test('profile parses preserve tasks on lock when enabled', () {
    final profile = Profile.fromJson({
      'profile': {'id': 'profile-id', 'display_name': 'Test'},
      'preserve_tasks_on_lock': true,
      'schedules': [],
      'daily_limits': [],
      'agent_users': [],
    });

    expect(profile.preserveTasksOnLock, isTrue);
  });
}
