class ProfileSummary {
  final String id;
  final String displayName;
  final int? remainingMinutes;
  final int? limitMinutes;
  final int usedMinutes;
  final String enforce;
  final int agentsOnline;
  final int agentsTotal;

  const ProfileSummary({
    required this.id,
    required this.displayName,
    this.remainingMinutes,
    this.limitMinutes,
    required this.usedMinutes,
    required this.enforce,
    required this.agentsOnline,
    required this.agentsTotal,
  });

  factory ProfileSummary.fromJson(Map<String, dynamic> j) => ProfileSummary(
        id: j['id'] as String,
        displayName: j['display_name'] as String,
        remainingMinutes: j['remaining_minutes'] as int?,
        limitMinutes: j['limit_minutes'] as int?,
        usedMinutes: (j['used_minutes'] as int?) ?? 0,
        enforce: (j['enforce'] as String?) ?? 'allow',
        agentsOnline: (j['agents_online'] as int?) ?? 0,
        agentsTotal: (j['agents_total'] as int?) ?? 0,
      );
}

class Profile {
  final String id;
  final String displayName;
  final String language;
  final List<Schedule> schedules;
  final List<DailyLimit> dailyLimits;
  final List<AgentUser> agentUsers;

  const Profile({
    required this.id,
    required this.displayName,
    required this.language,
    required this.schedules,
    required this.dailyLimits,
    required this.agentUsers,
  });

  factory Profile.fromJson(Map<String, dynamic> j) {
    final p = j['profile'] as Map<String, dynamic>;
    return Profile(
      id: p['id'] as String,
      displayName: p['display_name'] as String,
      language: (p['language'] as String?) ?? 'en',
      schedules: ((j['schedules'] as List?) ?? [])
          .map((s) => Schedule.fromJson(s as Map<String, dynamic>))
          .toList(),
      dailyLimits: ((j['daily_limits'] as List?) ?? [])
          .map((l) => DailyLimit.fromJson(l as Map<String, dynamic>))
          .toList(),
      agentUsers: ((j['agent_users'] as List?) ?? [])
          .map((u) => AgentUser.fromJson(u as Map<String, dynamic>))
          .toList(),
    );
  }
}

class Schedule {
  final String? id;
  final int dayOfWeek;
  final String startTime;
  final String endTime;

  const Schedule({
    this.id,
    required this.dayOfWeek,
    required this.startTime,
    required this.endTime,
  });

  factory Schedule.fromJson(Map<String, dynamic> j) => Schedule(
        id: j['id'] as String?,
        dayOfWeek: j['day_of_week'] as int,
        startTime: j['start_time'] as String,
        endTime: j['end_time'] as String,
      );

  Map<String, dynamic> toJson() => {
        'day_of_week': dayOfWeek,
        'start_time': startTime,
        'end_time': endTime,
      };

  Schedule copyWith({int? dayOfWeek, String? startTime, String? endTime}) => Schedule(
        id: id,
        dayOfWeek: dayOfWeek ?? this.dayOfWeek,
        startTime: startTime ?? this.startTime,
        endTime: endTime ?? this.endTime,
      );
}

class DailyLimit {
  final int dayOfWeek;
  final int? allowedMinutes;

  const DailyLimit({required this.dayOfWeek, this.allowedMinutes});

  factory DailyLimit.fromJson(Map<String, dynamic> j) => DailyLimit(
        dayOfWeek: j['day_of_week'] as int,
        allowedMinutes: j['allowed_minutes'] as int?,
      );

  Map<String, dynamic> toJson() => {
        'day_of_week': dayOfWeek,
        'allowed_minutes': allowedMinutes ?? 0,
      };
}

class AgentUser {
  final String id;
  final String agentId;
  final String? profileId;
  final int localUid;
  final String localUsername;
  final String? displayName;
  final String status;

  const AgentUser({
    required this.id,
    required this.agentId,
    this.profileId,
    required this.localUid,
    required this.localUsername,
    this.displayName,
    required this.status,
  });

  factory AgentUser.fromJson(Map<String, dynamic> j) => AgentUser(
        id: j['id'] as String,
        agentId: j['agent_id'] as String,
        profileId: j['profile_id'] as String?,
        localUid: j['local_uid'] as int,
        localUsername: j['local_username'] as String,
        displayName: j['display_name'] as String?,
        status: (j['status'] as String?) ?? 'unmanaged',
      );
}

class ProfileStatus {
  final String profileId;
  final String displayName;
  final TodayStatus today;
  final List<AgentOnlineStatus> agents;

  const ProfileStatus({
    required this.profileId,
    required this.displayName,
    required this.today,
    required this.agents,
  });

  factory ProfileStatus.fromJson(Map<String, dynamic> j) {
    final p = j['profile'] as Map<String, dynamic>;
    return ProfileStatus(
      profileId: p['id'] as String,
      displayName: p['display_name'] as String,
      today: TodayStatus.fromJson(p['today'] as Map<String, dynamic>),
      agents: ((p['agents'] as List?) ?? [])
          .map((a) => AgentOnlineStatus.fromJson(a as Map<String, dynamic>))
          .toList(),
    );
  }
}

class TodayStatus {
  final String date;
  final int? limitMinutes;
  final int usedMinutes;
  final int adjustmentsMinutes;
  final int remainingMinutes;
  final String enforce;

  const TodayStatus({
    required this.date,
    this.limitMinutes,
    required this.usedMinutes,
    required this.adjustmentsMinutes,
    required this.remainingMinutes,
    required this.enforce,
  });

  factory TodayStatus.fromJson(Map<String, dynamic> j) => TodayStatus(
        date: j['date'] as String,
        limitMinutes: j['limit_minutes'] as int?,
        usedMinutes: (j['used_minutes'] as int?) ?? 0,
        adjustmentsMinutes: (j['adjustments_minutes'] as int?) ?? 0,
        remainingMinutes: (j['remaining_minutes'] as int?) ?? 0,
        enforce: (j['enforce'] as String?) ?? 'allow',
      );
}

class AgentOnlineStatus {
  final String agentId;
  final String agentName;
  final String localUsername;
  final bool online;
  final int usedTodayMinutes;

  const AgentOnlineStatus({
    required this.agentId,
    required this.agentName,
    required this.localUsername,
    required this.online,
    required this.usedTodayMinutes,
  });

  factory AgentOnlineStatus.fromJson(Map<String, dynamic> j) => AgentOnlineStatus(
        agentId: j['agent_id'] as String,
        agentName: (j['agent_name'] as String?) ?? '',
        localUsername: (j['local_username'] as String?) ?? '',
        online: (j['online'] as bool?) ?? false,
        usedTodayMinutes: (j['used_today_minutes'] as int?) ?? 0,
      );
}

class UsageEntry {
  final String date;
  final int usedMinutes;
  final int? limitMinutes;
  final int adjustmentsMinutes;

  const UsageEntry({
    required this.date,
    required this.usedMinutes,
    this.limitMinutes,
    required this.adjustmentsMinutes,
  });

  factory UsageEntry.fromJson(Map<String, dynamic> j) => UsageEntry(
        date: j['date'] as String,
        usedMinutes: (j['used_minutes'] as int?) ?? 0,
        limitMinutes: j['limit_minutes'] as int?,
        adjustmentsMinutes: (j['adjustments_minutes'] as int?) ?? 0,
      );
}

class Agent {
  final String id;
  final String machineId;
  final String displayName;
  final String hostname;
  final String timezone;
  final String status;
  final bool online;
  final int? lastSeenAt;
  final String? agentVersion;
  final int userCount;
  final String? pairingCode;

  const Agent({
    required this.id,
    required this.machineId,
    required this.displayName,
    required this.hostname,
    required this.timezone,
    required this.status,
    required this.online,
    this.lastSeenAt,
    this.agentVersion,
    required this.userCount,
    this.pairingCode,
  });

  factory Agent.fromJson(Map<String, dynamic> j) => Agent(
        id: j['id'] as String,
        machineId: (j['machine_id'] as String?) ?? '',
        displayName: (j['display_name'] as String?) ?? '',
        hostname: (j['hostname'] as String?) ?? '',
        timezone: (j['timezone'] as String?) ?? 'UTC',
        status: (j['status'] as String?) ?? 'unknown',
        online: (j['online'] as bool?) ?? false,
        lastSeenAt: j['last_seen_at'] as int?,
        agentVersion: j['agent_version'] as String?,
        userCount: (j['user_count'] as int?) ?? 0,
        pairingCode: j['pairing_code'] as String?,
      );

  String get name => displayName.isNotEmpty ? displayName : hostname;
}
