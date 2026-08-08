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

  test('unlock reverses the current negative net adjustment', () {
    expect(unlockAdjustmentFor(-90), 90);
  });

  test('unlock is unavailable without a negative net adjustment', () {
    expect(unlockAdjustmentFor(0), 0);
    expect(unlockAdjustmentFor(30), 0);
  });
}
