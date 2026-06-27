import 'package:flutter/material.dart';

class AppLocalizations {
  final Locale locale;
  AppLocalizations(this.locale);

  static AppLocalizations of(BuildContext context) =>
      Localizations.of<AppLocalizations>(context, AppLocalizations) ??
      AppLocalizations(const Locale('en'));

  static const delegate = _AppLocalizationsDelegate();

  static const supportedLocales = [
    Locale('en'), Locale('pl'), Locale('es'),
    Locale('fr'), Locale('de'), Locale('pt'),
  ];

  String _t(String key) {
    final code = locale.languageCode;
    return (_strings[code] ?? _strings['en']!)[key] ?? _strings['en']![key] ?? key;
  }

  // Navigation
  String get profiles => _t('profiles');
  String get devices => _t('devices');
  String get settings => _t('settings');

  // Common
  String get cancel => _t('cancel');
  String get save => _t('save');
  String get confirm => _t('confirm');
  String get retry => _t('retry');
  String get signOut => _t('signOut');
  String get changeServer => _t('changeServer');
  String get create => _t('create');
  String get send => _t('send');
  String get connect => _t('connect');
  String get rename => _t('rename');
  String get delete => _t('delete');
  String get failedToLoad => _t('failedToLoad');
  String get now => _t('now');

  // Server setup
  String get connectToServer => _t('connectToServer');
  String get findServerDesc => _t('findServerDesc');
  String get nearbyServers => _t('nearbyServers');
  String get scan => _t('scan');
  String get noServersFound => _t('noServersFound');
  String get manualEntry => _t('manualEntry');
  String get serverAddress => _t('serverAddress');
  String get serverAddressHint => _t('serverAddressHint');
  String get connectManually => _t('connectManually');
  String cannotReachAt(String url) => _t('cannotReachAt').replaceAll('{url}', url);
  String get cannotReachAddr => _t('cannotReachAddr');

  // Login
  String get signIn => _t('signIn');
  String get createAdmin => _t('createAdmin');
  String get createAdminDesc => _t('createAdminDesc');
  String get signInDesc => _t('signInDesc');
  String get username => _t('username');
  String get password => _t('password');
  String get createAccount => _t('createAccount');
  String get connectionError => _t('connectionError');

  // Dashboard
  String get newProfile => _t('newProfile');
  String get profileNameLabel => _t('profileNameLabel');
  String get noProfilesYet => _t('noProfilesYet');
  String get locked => _t('locked');
  String get profileTitle => _t('profileTitle');
  String devicesWaiting(int n) => _t('devicesWaiting').replaceAll('{n}', '$n');
  String devicesOnline(int a, int b) =>
      _t('devicesOnline').replaceAll('{a}', '$a').replaceAll('{b}', '$b');
  String usedNoLimit(String u) => _t('usedNoLimit').replaceAll('{u}', u);
  String usedWithLimit(String u, String r, String l) =>
      _t('usedWithLimit').replaceAll('{u}', u).replaceAll('{r}', r).replaceAll('{l}', l);

  // Profile detail
  String get today => _t('today');
  String get noLimit => _t('noLimit');
  String get adjust => _t('adjust');
  String get reset => _t('reset');
  String get apply => _t('apply');
  String get lockNow => _t('lockNow');
  String get lockNowConfirmTitle => _t('lockNowConfirmTitle');
  String get lockNowConfirmBody => _t('lockNowConfirmBody');
  String get message => _t('message');
  String get sendMessage => _t('sendMessage');
  String get messageToUser => _t('messageToUser');
  String get messageSent => _t('messageSent');
  String get timeAdjusted => _t('timeAdjusted');
  String get screenTimeLocked => _t('screenTimeLocked');
  String usedOf(String u) => _t('usedOf').replaceAll('{u}', u);
  String usedOfLimit(String u, String l) =>
      _t('usedOfLimit').replaceAll('{u}', u).replaceAll('{l}', l);
  String timeAdjustment(String a) => _t('timeAdjustmentLabel').replaceAll('{a}', a);
  String get limitsSaved => _t('limitsSaved');
  String get scheduleSaved => _t('scheduleSaved');
  String get languageSaved => _t('languageSaved');
  String get notifLanguage => _t('notifLanguage');
  String get linkedAccounts => _t('linkedAccounts');
  String get renameProfile => _t('renameProfile');
  String get deleteProfileTitle => _t('deleteProfileTitle');
  String get deleteProfileBody => _t('deleteProfileBody');

  // Daily limits widget
  String get dailyLimits => _t('dailyLimits');
  String get dailyLimitsDesc => _t('dailyLimitsDesc');
  String get saveLimits => _t('saveLimits');
  String get noLimitShort => _t('noLimitShort');
  String get blocked => _t('blocked');
  String get block => _t('block');
  String get clear => _t('clear');
  String get addLimit => _t('addLimit');

  // Schedule widget
  String get timeWindows => _t('timeWindows');
  String get timeWindowsDesc => _t('timeWindowsDesc');
  String get noWindows => _t('noWindows');
  String get addWindow => _t('addWindow');
  String get saveWindows => _t('saveWindows');
  String get addTimeWindow => _t('addTimeWindow');
  String get editTimeWindow => _t('editTimeWindow');
  String get dayLabel => _t('dayLabel');
  String get start => _t('start');
  String get end => _t('end');

  // Usage chart
  String get chartUsage => _t('chartUsage');

  // Agents screen
  String get noDevicesYet => _t('noDevicesYet');
  String get noDevicesDesc => _t('noDevicesDesc');
  String userCount(int n) => _t('userCount').replaceAll('{n}', '$n');
  String get statusPending => _t('statusPending');
  String get statusRemoving => _t('statusRemoving');
  String get statusDisabled => _t('statusDisabled');
  String get statusOnline => _t('statusOnline');
  String get statusOffline => _t('statusOffline');

  // Agent detail
  String get deviceTitle => _t('deviceTitle');
  String get waitingApproval => _t('waitingApproval');
  String get pairingCode => _t('pairingCode');
  String get acceptDevice => _t('acceptDevice');
  String get managedUsers => _t('managedUsers');
  String get noUsersYet => _t('noUsersYet');
  String get renameDevice => _t('renameDevice');
  String get deleteDeviceTitle => _t('deleteDeviceTitle');
  String get deleteDeviceBody => _t('deleteDeviceBody');
  String get forceRemoveTitle => _t('forceRemoveTitle');
  String get forceRemoveBody => _t('forceRemoveBody');
  String get forceRemoveMenu => _t('forceRemoveMenu');
  String get userLinked => _t('userLinked');
  String get userUnlinked => _t('userUnlinked');
  String get deviceAccepted => _t('deviceAccepted');
  String get deletionCancelled => _t('deletionCancelled');
  String get unmanaged => _t('unmanaged');
  String get undoDeletion => _t('undoDeletion');
  String get copiedToClipboard => _t('copiedToClipboard');
  String get lastSeen => _t('lastSeen');
  String get timezone => _t('timezone');
  String get agentVersion => _t('agentVersion');
  String get machineId => _t('machineId');
  String get copyId => _t('copyId');
  String get statusPendingApproval => _t('statusPendingApproval');
  String get statusPendingRemoval => _t('statusPendingRemoval');

  // Agent logs
  String get recentLogs => _t('recentLogs');
  String get loadLogs => _t('loadLogs');
  String get refreshLogs => _t('refreshLogs');
  String get agentOfflineLogs => _t('agentOfflineLogs');
  String get logsEmpty => _t('logsEmpty');

  // Localized time ago
  String timeAgo(int? unixSeconds) {
    if (unixSeconds == null) return _t('never');
    final dt = DateTime.fromMillisecondsSinceEpoch(unixSeconds * 1000);
    final diff = DateTime.now().difference(dt);
    if (diff.inSeconds < 60) return _t('justNow');
    if (diff.inMinutes < 60) return _t('minutesAgo').replaceAll('{n}', '${diff.inMinutes}');
    if (diff.inHours < 24) return _t('hoursAgo').replaceAll('{n}', '${diff.inHours}');
    if (diff.inDays < 7) return _t('daysAgo').replaceAll('{n}', '${diff.inDays}');
    return '${dt.day}/${dt.month}';
  }

  // Day names
  String dayShort(int dow) => _t('day${dow.clamp(0, 6)}');
  String dayFull(int dow) => _t('dayFull${dow.clamp(0, 6)}');
  String dayLetter(int dow) => _t('dayLetter${dow.clamp(0, 6)}');

  // About
  String get about => _t('about');
  String get aboutTitle => _t('aboutTitle');
  String get appDescription => _t('appDescription');
  String get author => _t('author');
  String get version => _t('version');
  String get sourceCode => _t('sourceCode');
  String get licenses => _t('licenses');

  // Settings
  String get settingsTitle => _t('settingsTitle');
  String get appearance => _t('appearance');
  String get theme => _t('theme');
  String get themeSystem => _t('themeSystem');
  String get themeLight => _t('themeLight');
  String get themeDark => _t('themeDark');
  String get language => _t('language');
  String get appLanguage => _t('appLanguage');
  String get languageAuto => _t('languageAuto');
  String get account => _t('account');

  // ---------------------------------------------------------------------------
  static const _en = <String, String>{
    'profiles': 'Profiles', 'devices': 'Devices', 'settings': 'Settings',
    'cancel': 'Cancel', 'save': 'Save', 'confirm': 'Confirm',
    'retry': 'Retry', 'signOut': 'Sign out', 'changeServer': 'Change server',
    'create': 'Create', 'send': 'Send', 'connect': 'Connect',
    'rename': 'Rename', 'delete': 'Delete', 'failedToLoad': 'Failed to load',
    'now': 'Now',
    'connectToServer': 'Connect to server',
    'findServerDesc': 'Find your ScreenGuard server on the local network.',
    'nearbyServers': 'Nearby servers', 'scan': 'Scan',
    'noServersFound': 'No servers found on this network.',
    'manualEntry': 'Manual entry', 'serverAddress': 'Server address',
    'serverAddressHint': '192.168.1.10:8080 or https://host',
    'connectManually': 'Connect manually',
    'cannotReachAt': 'Cannot reach server at {url}',
    'cannotReachAddr': 'Cannot reach server. Check the address and port.',
    'signIn': 'Sign in', 'createAdmin': 'Create admin account',
    'createAdminDesc': 'Create administrator credentials for your server.',
    'signInDesc': 'Sign in to manage parental controls.',
    'username': 'Username', 'password': 'Password',
    'createAccount': 'Create account',
    'connectionError': 'Connection error. Please try again.',
    'newProfile': 'New profile', 'profileNameLabel': 'Name',
    'noProfilesYet': 'No profiles yet. Tap + to create one.',
    'locked': 'Locked', 'profileTitle': 'Profile',
    'devicesWaiting': '{n} device(s) waiting for approval',
    'devicesOnline': '{a}/{b} devices online',
    'usedNoLimit': '{u} used · no limit',
    'usedWithLimit': '{u} used · {r} left of {l}',
    'today': 'Today', 'noLimit': 'No limit',
    'adjust': 'Adjust', 'reset': 'Reset', 'apply': 'Apply',
    'lockNow': 'Lock now',
    'lockNowConfirmTitle': 'Lock now?',
    'lockNowConfirmBody': "This will zero out today's remaining screen time.",
    'message': 'Message', 'sendMessage': 'Send message',
    'messageToUser': 'Message to user…',
    'messageSent': 'Message sent', 'timeAdjusted': 'Time adjusted',
    'screenTimeLocked': 'Screen time locked',
    'usedOf': '{u} used', 'usedOfLimit': '{u} used of {l}',
    'timeAdjustmentLabel': '{a} adjustment',
    'limitsSaved': 'Limits saved', 'scheduleSaved': 'Schedule saved',
    'languageSaved': 'Language saved',
    'notifLanguage': 'Notification language',
    'linkedAccounts': 'Linked accounts',
    'renameProfile': 'Rename profile',
    'deleteProfileTitle': 'Delete profile?',
    'deleteProfileBody': 'This will permanently delete the profile and all its settings.',
    'dailyLimits': 'Daily limits',
    'dailyLimitsDesc': 'Max screen time per day. Leave blank for no limit.',
    'saveLimits': 'Save limits',
    'noLimitShort': 'No limit', 'blocked': 'Blocked',
    'block': 'Block', 'clear': 'Clear', 'addLimit': 'Add limit',
    'timeWindows': 'Allowed time windows',
    'timeWindowsDesc': 'Screen time is only allowed during these windows.',
    'noWindows': 'No windows set — all hours allowed.',
    'addWindow': 'Add window', 'saveWindows': 'Save windows',
    'addTimeWindow': 'Add time window', 'editTimeWindow': 'Edit time window',
    'dayLabel': 'Day', 'start': 'Start', 'end': 'End',
    'chartUsage': 'Usage',
    'noDevicesYet': 'No devices registered yet',
    'noDevicesDesc': 'Install the agent on a managed computer to get started.',
    'userCount': '{n} user(s)',
    'statusPending': 'Pending', 'statusRemoving': 'Removing',
    'statusDisabled': 'Disabled', 'statusOnline': 'Online', 'statusOffline': 'Offline',
    'deviceTitle': 'Device',
    'waitingApproval': 'Waiting for approval',
    'pairingCode': 'Pairing code', 'acceptDevice': 'Accept device',
    'managedUsers': 'Managed users', 'noUsersYet': 'No users discovered yet.',
    'renameDevice': 'Rename device',
    'deleteDeviceTitle': 'Delete device?',
    'deleteDeviceBody': 'The device will be unlinked the next time it connects.',
    'forceRemoveTitle': 'Force remove?',
    'forceRemoveBody': 'The device record will be permanently deleted from the server immediately.',
    'forceRemoveMenu': 'Force remove',
    'userLinked': 'User linked', 'userUnlinked': 'User unlinked',
    'deviceAccepted': 'Device accepted', 'deletionCancelled': 'Deletion cancelled',
    'unmanaged': 'Unmanaged', 'undoDeletion': 'Undo deletion',
    'copiedToClipboard': 'Copied to clipboard',
    'lastSeen': 'Last seen', 'timezone': 'Timezone',
    'agentVersion': 'Version', 'machineId': 'Machine ID', 'copyId': 'Copy ID',
    'statusPendingApproval': 'Pending approval', 'statusPendingRemoval': 'Pending removal',
    'recentLogs': 'Recent logs', 'loadLogs': 'Load logs', 'refreshLogs': 'Refresh',
    'agentOfflineLogs': 'Agent is offline — connect it to load logs',
    'logsEmpty': 'No log lines returned',
    'never': 'never', 'justNow': 'just now',
    'minutesAgo': '{n}m ago', 'hoursAgo': '{n}h ago', 'daysAgo': '{n}d ago',
    'day0': 'Mon', 'day1': 'Tue', 'day2': 'Wed', 'day3': 'Thu',
    'day4': 'Fri', 'day5': 'Sat', 'day6': 'Sun',
    'dayFull0': 'Monday', 'dayFull1': 'Tuesday', 'dayFull2': 'Wednesday',
    'dayFull3': 'Thursday', 'dayFull4': 'Friday', 'dayFull5': 'Saturday', 'dayFull6': 'Sunday',
    'dayLetter0': 'M', 'dayLetter1': 'T', 'dayLetter2': 'W', 'dayLetter3': 'T',
    'dayLetter4': 'F', 'dayLetter5': 'S', 'dayLetter6': 'S',
    'settingsTitle': 'Settings', 'appearance': 'Appearance', 'theme': 'Theme',
    'themeSystem': 'System', 'themeLight': 'Light', 'themeDark': 'Dark',
    'language': 'Language', 'appLanguage': 'App language',
    'languageAuto': 'System (auto)', 'account': 'Account',
    'about': 'About', 'aboutTitle': 'About ScreenGuard',
    'appDescription': 'Parental control software for managing screen time and device usage.',
    'author': 'Author', 'version': 'Version',
    'sourceCode': 'Source code', 'licenses': 'Licenses',
  };

  static const _pl = <String, String>{
    'profiles': 'Profile', 'devices': 'Urządzenia', 'settings': 'Ustawienia',
    'cancel': 'Anuluj', 'save': 'Zapisz', 'confirm': 'Potwierdź',
    'retry': 'Ponów', 'signOut': 'Wyloguj', 'changeServer': 'Zmień serwer',
    'create': 'Utwórz', 'send': 'Wyślij', 'connect': 'Połącz',
    'rename': 'Zmień nazwę', 'delete': 'Usuń', 'failedToLoad': 'Błąd ładowania',
    'now': 'Teraz',
    'connectToServer': 'Połącz z serwerem',
    'findServerDesc': 'Znajdź serwer ScreenGuard w sieci lokalnej.',
    'nearbyServers': 'Serwery w pobliżu', 'scan': 'Skanuj',
    'noServersFound': 'Nie znaleziono serwerów w tej sieci.',
    'manualEntry': 'Ręczne wprowadzanie', 'serverAddress': 'Adres serwera',
    'serverAddressHint': '192.168.1.10:8080 lub https://host',
    'connectManually': 'Połącz ręcznie',
    'cannotReachAt': 'Nie można połączyć z serwerem {url}',
    'cannotReachAddr': 'Nie można połączyć z serwerem. Sprawdź adres i port.',
    'signIn': 'Zaloguj się', 'createAdmin': 'Utwórz konto administratora',
    'createAdminDesc': 'Utwórz dane logowania administratora dla serwera.',
    'signInDesc': 'Zaloguj się, aby zarządzać kontrolą rodzicielską.',
    'username': 'Nazwa użytkownika', 'password': 'Hasło',
    'createAccount': 'Utwórz konto',
    'connectionError': 'Błąd połączenia. Spróbuj ponownie.',
    'newProfile': 'Nowy profil', 'profileNameLabel': 'Nazwa',
    'noProfilesYet': 'Brak profili. Naciśnij +, aby utworzyć.',
    'locked': 'Zablokowany', 'profileTitle': 'Profil',
    'devicesWaiting': '{n} urządzenie(a) oczekuje na zatwierdzenie',
    'devicesOnline': '{a}/{b} urządzeń online',
    'usedNoLimit': '{u} użyto · bez limitu',
    'usedWithLimit': '{u} użyto · {r} pozostało z {l}',
    'today': 'Dziś', 'noLimit': 'Bez limitu',
    'adjust': 'Dostosuj', 'reset': 'Resetuj', 'apply': 'Zastosuj',
    'lockNow': 'Zablokuj teraz',
    'lockNowConfirmTitle': 'Zablokować teraz?',
    'lockNowConfirmBody': 'Spowoduje to wyzerowanie dzisiejszego pozostałego czasu ekranu.',
    'message': 'Wiadomość', 'sendMessage': 'Wyślij wiadomość',
    'messageToUser': 'Wiadomość do użytkownika…',
    'messageSent': 'Wiadomość wysłana', 'timeAdjusted': 'Czas dostosowany',
    'screenTimeLocked': 'Czas ekranu zablokowany',
    'usedOf': 'użyto {u}', 'usedOfLimit': 'użyto {u} z {l}',
    'timeAdjustmentLabel': 'korekta {a}',
    'limitsSaved': 'Limity zapisane', 'scheduleSaved': 'Harmonogram zapisany',
    'languageSaved': 'Język zapisany',
    'notifLanguage': 'Język powiadomień',
    'linkedAccounts': 'Powiązane konta',
    'renameProfile': 'Zmień nazwę profilu',
    'deleteProfileTitle': 'Usunąć profil?',
    'deleteProfileBody': 'Spowoduje to trwałe usunięcie profilu i wszystkich jego ustawień.',
    'dailyLimits': 'Dzienne limity',
    'dailyLimitsDesc': 'Maksymalny czas ekranu na dzień. Zostaw puste, aby bez limitu.',
    'saveLimits': 'Zapisz limity',
    'noLimitShort': 'Bez limitu', 'blocked': 'Zablokowany',
    'block': 'Blokuj', 'clear': 'Wyczyść', 'addLimit': 'Dodaj limit',
    'timeWindows': 'Dozwolone okna czasowe',
    'timeWindowsDesc': 'Czas ekranu dozwolony tylko w tych oknach.',
    'noWindows': 'Brak okien — wszystkie godziny dozwolone.',
    'addWindow': 'Dodaj okno', 'saveWindows': 'Zapisz okna',
    'addTimeWindow': 'Dodaj okno czasowe', 'editTimeWindow': 'Edytuj okno czasowe',
    'dayLabel': 'Dzień', 'start': 'Początek', 'end': 'Koniec',
    'chartUsage': 'Użycie',
    'noDevicesYet': 'Nie zarejestrowano żadnych urządzeń',
    'noDevicesDesc': 'Zainstaluj agenta na zarządzanym komputerze, aby rozpo-cząć.',
    'userCount': '{n} użytkownik(ów)',
    'statusPending': 'Oczekuje', 'statusRemoving': 'Usuwanie',
    'statusDisabled': 'Wyłączony', 'statusOnline': 'Online', 'statusOffline': 'Offline',
    'deviceTitle': 'Urządzenie',
    'waitingApproval': 'Oczekuje na zatwierdzenie',
    'pairingCode': 'Kod parowania', 'acceptDevice': 'Zaakceptuj urządzenie',
    'managedUsers': 'Zarządzani użytkownicy', 'noUsersYet': 'Nie wykryto żadnych użytkowników.',
    'renameDevice': 'Zmień nazwę urządzenia',
    'deleteDeviceTitle': 'Usunąć urządzenie?',
    'deleteDeviceBody': 'Urządzenie zostanie odłączone przy następnym połączeniu.',
    'forceRemoveTitle': 'Wymuszone usunięcie?',
    'forceRemoveBody': 'Rekord urządzenia zostanie natychmiast trwale usunięty z serwera.',
    'forceRemoveMenu': 'Usuń przymusowo',
    'userLinked': 'Użytkownik powiązany', 'userUnlinked': 'Użytkownik odpowiązany',
    'deviceAccepted': 'Urządzenie zaakceptowane', 'deletionCancelled': 'Usunięcie anulowane',
    'unmanaged': 'Niezarządzany', 'undoDeletion': 'Cofnij usunięcie',
    'copiedToClipboard': 'Skopiowano do schowka',
    'lastSeen': 'Ostatnio widziany', 'timezone': 'Strefa czasowa',
    'agentVersion': 'Wersja', 'machineId': 'ID maszyny', 'copyId': 'Kopiuj ID',
    'statusPendingApproval': 'Oczekuje na zatwierdzenie', 'statusPendingRemoval': 'Oczekuje na usunięcie',
    'recentLogs': 'Ostatnie logi', 'loadLogs': 'Załaduj logi', 'refreshLogs': 'Odśwież',
    'agentOfflineLogs': 'Agent jest offline — połącz go, aby załadować logi',
    'logsEmpty': 'Brak wierszy dziennika',
    'never': 'nigdy', 'justNow': 'przed chwilą',
    'minutesAgo': '{n} min temu', 'hoursAgo': '{n} h temu', 'daysAgo': '{n} dni temu',
    'day0': 'Pon', 'day1': 'Wt', 'day2': 'Śr', 'day3': 'Czw',
    'day4': 'Pt', 'day5': 'Sob', 'day6': 'Niedz',
    'dayFull0': 'Poniedziałek', 'dayFull1': 'Wtorek', 'dayFull2': 'Środa',
    'dayFull3': 'Czwartek', 'dayFull4': 'Piątek', 'dayFull5': 'Sobota', 'dayFull6': 'Niedziela',
    'dayLetter0': 'P', 'dayLetter1': 'W', 'dayLetter2': 'Ś', 'dayLetter3': 'C',
    'dayLetter4': 'P', 'dayLetter5': 'S', 'dayLetter6': 'N',
    'settingsTitle': 'Ustawienia', 'appearance': 'Wygląd', 'theme': 'Motyw',
    'themeSystem': 'Systemowy', 'themeLight': 'Jasny', 'themeDark': 'Ciemny',
    'language': 'Język', 'appLanguage': 'Język aplikacji',
    'languageAuto': 'Systemowy (auto)', 'account': 'Konto',
    'about': 'O aplikacji', 'aboutTitle': 'O ScreenGuard',
    'appDescription': 'Oprogramowanie do kontroli rodzicielskiej do zarządzania czasem ekranu i użytkowaniem urządzeń.',
    'author': 'Autor', 'version': 'Wersja',
    'sourceCode': 'Kod źródłowy', 'licenses': 'Licencje',
  };

  static const _es = <String, String>{
    'profiles': 'Perfiles', 'devices': 'Dispositivos', 'settings': 'Configuración',
    'cancel': 'Cancelar', 'save': 'Guardar', 'confirm': 'Confirmar',
    'retry': 'Reintentar', 'signOut': 'Cerrar sesión', 'changeServer': 'Cambiar servidor',
    'create': 'Crear', 'send': 'Enviar', 'connect': 'Conectar',
    'rename': 'Renombrar', 'delete': 'Eliminar', 'failedToLoad': 'Error al cargar',
    'now': 'Ahora',
    'connectToServer': 'Conectar al servidor',
    'findServerDesc': 'Encuentra tu servidor ScreenGuard en la red local.',
    'nearbyServers': 'Servidores cercanos', 'scan': 'Escanear',
    'noServersFound': 'No se encontraron servidores en esta red.',
    'manualEntry': 'Entrada manual', 'serverAddress': 'Dirección del servidor',
    'serverAddressHint': '192.168.1.10:8080 o https://host',
    'connectManually': 'Conectar manualmente',
    'cannotReachAt': 'No se puede conectar al servidor en {url}',
    'cannotReachAddr': 'No se puede conectar al servidor. Verifica la dirección y el puerto.',
    'signIn': 'Iniciar sesión', 'createAdmin': 'Crear cuenta de administrador',
    'createAdminDesc': 'Crea credenciales de administrador para tu servidor.',
    'signInDesc': 'Inicia sesión para gestionar el control parental.',
    'username': 'Usuario', 'password': 'Contraseña',
    'createAccount': 'Crear cuenta',
    'connectionError': 'Error de conexión. Inténtalo de nuevo.',
    'newProfile': 'Nuevo perfil', 'profileNameLabel': 'Nombre',
    'noProfilesYet': 'Sin perfiles. Toca + para crear uno.',
    'locked': 'Bloqueado', 'profileTitle': 'Perfil',
    'devicesWaiting': '{n} dispositivo(s) esperando aprobación',
    'devicesOnline': '{a}/{b} dispositivos en línea',
    'usedNoLimit': '{u} usado · sin límite',
    'usedWithLimit': '{u} usado · {r} restante de {l}',
    'today': 'Hoy', 'noLimit': 'Sin límite',
    'adjust': 'Ajustar', 'reset': 'Restablecer', 'apply': 'Aplicar',
    'lockNow': 'Bloquear ahora',
    'lockNowConfirmTitle': '¿Bloquear ahora?',
    'lockNowConfirmBody': 'Esto pondrá a cero el tiempo de pantalla restante de hoy.',
    'message': 'Mensaje', 'sendMessage': 'Enviar mensaje',
    'messageToUser': 'Mensaje al usuario…',
    'messageSent': 'Mensaje enviado', 'timeAdjusted': 'Tiempo ajustado',
    'screenTimeLocked': 'Tiempo de pantalla bloqueado',
    'usedOf': '{u} usado', 'usedOfLimit': '{u} usado de {l}',
    'timeAdjustmentLabel': 'ajuste {a}',
    'limitsSaved': 'Límites guardados', 'scheduleSaved': 'Horario guardado',
    'languageSaved': 'Idioma guardado',
    'notifLanguage': 'Idioma de notificaciones',
    'linkedAccounts': 'Cuentas vinculadas',
    'renameProfile': 'Renombrar perfil',
    'deleteProfileTitle': '¿Eliminar perfil?',
    'deleteProfileBody': 'Esto eliminará permanentemente el perfil y toda su configuración.',
    'dailyLimits': 'Límites diarios',
    'dailyLimitsDesc': 'Tiempo máximo de pantalla por día. Deja en blanco para sin límite.',
    'saveLimits': 'Guardar límites',
    'noLimitShort': 'Sin límite', 'blocked': 'Bloqueado',
    'block': 'Bloquear', 'clear': 'Borrar', 'addLimit': 'Agregar límite',
    'timeWindows': 'Ventanas de tiempo permitidas',
    'timeWindowsDesc': 'El tiempo de pantalla solo se permite en estas ventanas.',
    'noWindows': 'Sin ventanas — todas las horas permitidas.',
    'addWindow': 'Agregar ventana', 'saveWindows': 'Guardar ventanas',
    'addTimeWindow': 'Agregar ventana de tiempo', 'editTimeWindow': 'Editar ventana de tiempo',
    'dayLabel': 'Día', 'start': 'Inicio', 'end': 'Fin',
    'chartUsage': 'Uso',
    'noDevicesYet': 'No hay dispositivos registrados',
    'noDevicesDesc': 'Instala el agente en un ordenador gestionado para empezar.',
    'userCount': '{n} usuario(s)',
    'statusPending': 'Pendiente', 'statusRemoving': 'Eliminando',
    'statusDisabled': 'Desactivado', 'statusOnline': 'En línea', 'statusOffline': 'Sin conexión',
    'deviceTitle': 'Dispositivo',
    'waitingApproval': 'Esperando aprobación',
    'pairingCode': 'Código de emparejamiento', 'acceptDevice': 'Aceptar dispositivo',
    'managedUsers': 'Usuarios gestionados', 'noUsersYet': 'No se han detectado usuarios aún.',
    'renameDevice': 'Renombrar dispositivo',
    'deleteDeviceTitle': '¿Eliminar dispositivo?',
    'deleteDeviceBody': 'El dispositivo se desvinculará la próxima vez que se conecte.',
    'forceRemoveTitle': '¿Eliminar por la fuerza?',
    'forceRemoveBody': 'El registro del dispositivo se eliminará permanentemente del servidor de inmediato.',
    'forceRemoveMenu': 'Eliminar forzosamente',
    'userLinked': 'Usuario vinculado', 'userUnlinked': 'Usuario desvinculado',
    'deviceAccepted': 'Dispositivo aceptado', 'deletionCancelled': 'Eliminación cancelada',
    'unmanaged': 'No gestionado', 'undoDeletion': 'Deshacer eliminación',
    'copiedToClipboard': 'Copiado al portapapeles',
    'lastSeen': 'Última vez visto', 'timezone': 'Zona horaria',
    'agentVersion': 'Versión', 'machineId': 'ID de máquina', 'copyId': 'Copiar ID',
    'statusPendingApproval': 'Aprobación pendiente', 'statusPendingRemoval': 'Eliminación pendiente',
    'recentLogs': 'Registros recientes', 'loadLogs': 'Cargar registros', 'refreshLogs': 'Actualizar',
    'agentOfflineLogs': 'El agente está desconectado — conéctalo para cargar registros',
    'logsEmpty': 'No se devolvieron líneas de registro',
    'never': 'nunca', 'justNow': 'ahora mismo',
    'minutesAgo': 'hace {n}m', 'hoursAgo': 'hace {n}h', 'daysAgo': 'hace {n}d',
    'day0': 'Lun', 'day1': 'Mar', 'day2': 'Mié', 'day3': 'Jue',
    'day4': 'Vie', 'day5': 'Sáb', 'day6': 'Dom',
    'dayFull0': 'Lunes', 'dayFull1': 'Martes', 'dayFull2': 'Miércoles',
    'dayFull3': 'Jueves', 'dayFull4': 'Viernes', 'dayFull5': 'Sábado', 'dayFull6': 'Domingo',
    'dayLetter0': 'L', 'dayLetter1': 'M', 'dayLetter2': 'X', 'dayLetter3': 'J',
    'dayLetter4': 'V', 'dayLetter5': 'S', 'dayLetter6': 'D',
    'settingsTitle': 'Configuración', 'appearance': 'Apariencia', 'theme': 'Tema',
    'themeSystem': 'Sistema', 'themeLight': 'Claro', 'themeDark': 'Oscuro',
    'language': 'Idioma', 'appLanguage': 'Idioma de la aplicación',
    'languageAuto': 'Sistema (auto)', 'account': 'Cuenta',
    'about': 'Acerca de', 'aboutTitle': 'Acerca de ScreenGuard',
    'appDescription': 'Software de control parental para gestionar el tiempo de pantalla y el uso de dispositivos.',
    'author': 'Autor', 'version': 'Versión',
    'sourceCode': 'Código fuente', 'licenses': 'Licencias',
  };

  static const _fr = <String, String>{
    'profiles': 'Profils', 'devices': 'Appareils', 'settings': 'Paramètres',
    'cancel': 'Annuler', 'save': 'Enregistrer', 'confirm': 'Confirmer',
    'retry': 'Réessayer', 'signOut': 'Se déconnecter', 'changeServer': 'Changer de serveur',
    'create': 'Créer', 'send': 'Envoyer', 'connect': 'Connecter',
    'rename': 'Renommer', 'delete': 'Supprimer', 'failedToLoad': 'Échec du chargement',
    'now': 'Maintenant',
    'connectToServer': 'Se connecter au serveur',
    'findServerDesc': 'Trouvez votre serveur ScreenGuard sur le réseau local.',
    'nearbyServers': 'Serveurs à proximité', 'scan': 'Scanner',
    'noServersFound': 'Aucun serveur trouvé sur ce réseau.',
    'manualEntry': 'Saisie manuelle', 'serverAddress': 'Adresse du serveur',
    'serverAddressHint': '192.168.1.10:8080 ou https://hôte',
    'connectManually': 'Se connecter manuellement',
    'cannotReachAt': 'Impossible de joindre le serveur à {url}',
    'cannotReachAddr': 'Impossible de joindre le serveur. Vérifiez l’adresse et le port.',
    'signIn': 'Se connecter', 'createAdmin': 'Créer un compte administrateur',
    'createAdminDesc': 'Créez des identifiants administrateur pour votre serveur.',
    'signInDesc': 'Connectez-vous pour gérer le contrôle parental.',
    'username': "Nom d’utilisateur", 'password': 'Mot de passe',
    'createAccount': 'Créer un compte',
    'connectionError': 'Erreur de connexion. Veuillez réessayer.',
    'newProfile': 'Nouveau profil', 'profileNameLabel': 'Nom',
    'noProfilesYet': 'Aucun profil. Appuyez sur + pour en créer un.',
    'locked': 'Verrouillé', 'profileTitle': 'Profil',
    'devicesWaiting': "{n} appareil(s) en attente d’approbation",
    'devicesOnline': '{a}/{b} appareils en ligne',
    'usedNoLimit': '{u} utilisé · sans limite',
    'usedWithLimit': '{u} utilisé · {r} restant sur {l}',
    'today': "Aujourd’hui", 'noLimit': 'Sans limite',
    'adjust': 'Ajuster', 'reset': 'Réinitialiser', 'apply': 'Appliquer',
    'lockNow': 'Verrouiller maintenant',
    'lockNowConfirmTitle': 'Verrouiller maintenant ?',
    'lockNowConfirmBody': "Cela remettra à zéro le temps d’écran restant d’aujourd’hui.",
    'message': 'Message', 'sendMessage': 'Envoyer un message',
    'messageToUser': "Message à l’utilisateur…",
    'messageSent': 'Message envoyé', 'timeAdjusted': 'Temps ajusté',
    'screenTimeLocked': "Temps d’écran verrouillé",
    'usedOf': '{u} utilisé', 'usedOfLimit': '{u} utilisé sur {l}',
    'timeAdjustmentLabel': 'ajustement {a}',
    'limitsSaved': 'Limites enregistrées', 'scheduleSaved': 'Programme enregistré',
    'languageSaved': 'Langue enregistrée',
    'notifLanguage': 'Langue des notifications',
    'linkedAccounts': 'Comptes liés',
    'renameProfile': 'Renommer le profil',
    'deleteProfileTitle': 'Supprimer le profil ?',
    'deleteProfileBody': 'Cela supprimera définitivement le profil et tous ses paramètres.',
    'dailyLimits': 'Limites quotidiennes',
    'dailyLimitsDesc': "Temps d’écran maximum par jour. Laissez vide pour sans limite.",
    'saveLimits': 'Enregistrer les limites',
    'noLimitShort': 'Sans limite', 'blocked': 'Bloqué',
    'block': 'Bloquer', 'clear': 'Effacer', 'addLimit': 'Ajouter une limite',
    'timeWindows': 'Plages horaires autorisées',
    'timeWindowsDesc': "Le temps d’écran n’est autorisé que pendant ces plages.",
    'noWindows': 'Aucune plage définie — toutes les heures autorisées.',
    'addWindow': 'Ajouter une plage', 'saveWindows': 'Enregistrer les plages',
    'addTimeWindow': 'Ajouter une plage horaire', 'editTimeWindow': 'Modifier la plage horaire',
    'dayLabel': 'Jour', 'start': 'Début', 'end': 'Fin',
    'chartUsage': 'Utilisation',
    'noDevicesYet': 'Aucun appareil enregistré',
    'noDevicesDesc': "Installez l’agent sur un ordinateur géré pour commencer.",
    'userCount': '{n} utilisateur(s)',
    'statusPending': 'En attente', 'statusRemoving': 'Suppression',
    'statusDisabled': 'Désactivé', 'statusOnline': 'En ligne', 'statusOffline': 'Hors ligne',
    'deviceTitle': 'Appareil',
    'waitingApproval': "En attente d’approbation",
    'pairingCode': 'Code de jumelage', 'acceptDevice': "Accepter l’appareil",
    'managedUsers': 'Utilisateurs gérés', 'noUsersYet': 'Aucun utilisateur détecté.',
    'renameDevice': "Renommer l’appareil",
    'deleteDeviceTitle': "Supprimer l’appareil ?",
    'deleteDeviceBody': "L’appareil sera dissocié lors de sa prochaine connexion.",
    'forceRemoveTitle': 'Suppression forcée ?',
    'forceRemoveBody': "L’enregistrement de l’appareil sera immédiatement supprimé définitivement du serveur.",
    'forceRemoveMenu': 'Suppression forcée',
    'userLinked': 'Utilisateur lié', 'userUnlinked': 'Utilisateur dissocié',
    'deviceAccepted': 'Appareil accepté', 'deletionCancelled': 'Suppression annulée',
    'unmanaged': 'Non géré', 'undoDeletion': 'Annuler la suppression',
    'copiedToClipboard': 'Copié dans le presse-papiers',
    'lastSeen': 'Dernière connexion', 'timezone': 'Fuseau horaire',
    'agentVersion': 'Version', 'machineId': 'ID machine', 'copyId': "Copier l’ID",
    'statusPendingApproval': "Approbation en attente", 'statusPendingRemoval': "Suppression en attente",
    'recentLogs': 'Journaux récents', 'loadLogs': 'Charger les journaux', 'refreshLogs': 'Actualiser',
    'agentOfflineLogs': "L'agent est hors ligne — connectez-le pour charger les journaux",
    'logsEmpty': 'Aucune ligne de journal retournée',
    'never': 'jamais', 'justNow': "à l’instant",
    'minutesAgo': 'il y a {n}m', 'hoursAgo': 'il y a {n}h', 'daysAgo': 'il y a {n}j',
    'day0': 'Lun', 'day1': 'Mar', 'day2': 'Mer', 'day3': 'Jeu',
    'day4': 'Ven', 'day5': 'Sam', 'day6': 'Dim',
    'dayFull0': 'Lundi', 'dayFull1': 'Mardi', 'dayFull2': 'Mercredi',
    'dayFull3': 'Jeudi', 'dayFull4': 'Vendredi', 'dayFull5': 'Samedi', 'dayFull6': 'Dimanche',
    'dayLetter0': 'L', 'dayLetter1': 'M', 'dayLetter2': 'M', 'dayLetter3': 'J',
    'dayLetter4': 'V', 'dayLetter5': 'S', 'dayLetter6': 'D',
    'settingsTitle': 'Paramètres', 'appearance': 'Apparence', 'theme': 'Thème',
    'themeSystem': 'Système', 'themeLight': 'Clair', 'themeDark': 'Sombre',
    'language': 'Langue', 'appLanguage': "Langue de l’application",
    'languageAuto': 'Système (auto)', 'account': 'Compte',
    'about': 'À propos', 'aboutTitle': 'À propos de ScreenGuard',
    'appDescription': 'Logiciel de contrôle parental pour gérer le temps d\'écran et l\'utilisation des appareils.',
    'author': 'Auteur', 'version': 'Version',
    'sourceCode': 'Code source', 'licenses': 'Licences',
  };

  static const _de = <String, String>{
    'profiles': 'Profile', 'devices': 'Geräte', 'settings': 'Einstellungen',
    'cancel': 'Abbrechen', 'save': 'Speichern', 'confirm': 'Bestätigen',
    'retry': 'Wiederholen', 'signOut': 'Abmelden', 'changeServer': 'Server wechseln',
    'create': 'Erstellen', 'send': 'Senden', 'connect': 'Verbinden',
    'rename': 'Umbenennen', 'delete': 'Löschen', 'failedToLoad': 'Laden fehlgeschlagen',
    'now': 'Jetzt',
    'connectToServer': 'Mit Server verbinden',
    'findServerDesc': 'Finden Sie Ihren ScreenGuard-Server im lokalen Netzwerk.',
    'nearbyServers': 'Server in der Nähe', 'scan': 'Scannen',
    'noServersFound': 'Keine Server in diesem Netzwerk gefunden.',
    'manualEntry': 'Manuelle Eingabe', 'serverAddress': 'Serveradresse',
    'serverAddressHint': '192.168.1.10:8080 oder https://host',
    'connectManually': 'Manuell verbinden',
    'cannotReachAt': 'Server unter {url} nicht erreichbar',
    'cannotReachAddr': 'Server nicht erreichbar. Adresse und Port prüfen.',
    'signIn': 'Anmelden', 'createAdmin': 'Administrator-Konto erstellen',
    'createAdminDesc': 'Erstellen Sie Administrator-Anmeldedaten für Ihren Server.',
    'signInDesc': 'Melden Sie sich an, um die Kindersicherung zu verwalten.',
    'username': 'Benutzername', 'password': 'Passwort',
    'createAccount': 'Konto erstellen',
    'connectionError': 'Verbindungsfehler. Bitte erneut versuchen.',
    'newProfile': 'Neues Profil', 'profileNameLabel': 'Name',
    'noProfilesYet': 'Keine Profile. Tippen Sie auf +, um eines zu erstellen.',
    'locked': 'Gesperrt', 'profileTitle': 'Profil',
    'devicesWaiting': '{n} Gerät(e) warten auf Genehmigung',
    'devicesOnline': '{a}/{b} Geräte online',
    'usedNoLimit': '{u} genutzt · kein Limit',
    'usedWithLimit': '{u} genutzt · {r} verbleibend von {l}',
    'today': 'Heute', 'noLimit': 'Kein Limit',
    'adjust': 'Anpassen', 'reset': 'Zurücksetzen', 'apply': 'Anwenden',
    'lockNow': 'Jetzt sperren',
    'lockNowConfirmTitle': 'Jetzt sperren?',
    'lockNowConfirmBody': 'Damit wird die verbleibende Bildschirmzeit von heute auf null gesetzt.',
    'message': 'Nachricht', 'sendMessage': 'Nachricht senden',
    'messageToUser': 'Nachricht an Benutzer…',
    'messageSent': 'Nachricht gesendet', 'timeAdjusted': 'Zeit angepasst',
    'screenTimeLocked': 'Bildschirmzeit gesperrt',
    'usedOf': '{u} genutzt', 'usedOfLimit': '{u} genutzt von {l}',
    'timeAdjustmentLabel': 'Anpassung {a}',
    'limitsSaved': 'Limits gespeichert', 'scheduleSaved': 'Zeitplan gespeichert',
    'languageSaved': 'Sprache gespeichert',
    'notifLanguage': 'Benachrichtigungssprache',
    'linkedAccounts': 'Verknüpfte Konten',
    'renameProfile': 'Profil umbenennen',
    'deleteProfileTitle': 'Profil löschen?',
    'deleteProfileBody': 'Das Profil und alle zugehörigen Einstellungen werden dauerhaft gelöscht.',
    'dailyLimits': 'Tägliche Limits',
    'dailyLimitsDesc': 'Maximale Bildschirmzeit pro Tag. Leer lassen für kein Limit.',
    'saveLimits': 'Limits speichern',
    'noLimitShort': 'Kein Limit', 'blocked': 'Gesperrt',
    'block': 'Sperren', 'clear': 'Löschen', 'addLimit': 'Limit hinzufügen',
    'timeWindows': 'Erlaubte Zeitfenster',
    'timeWindowsDesc': 'Bildschirmzeit ist nur in diesen Fenstern erlaubt.',
    'noWindows': 'Keine Fenster gesetzt — alle Stunden erlaubt.',
    'addWindow': 'Fenster hinzufügen', 'saveWindows': 'Fenster speichern',
    'addTimeWindow': 'Zeitfenster hinzufügen', 'editTimeWindow': 'Zeitfenster bearbeiten',
    'dayLabel': 'Tag', 'start': 'Beginn', 'end': 'Ende',
    'chartUsage': 'Nutzung',
    'noDevicesYet': 'Noch keine Geräte registriert',
    'noDevicesDesc': 'Installieren Sie den Agent auf einem verwalteten Computer, um zu beginnen.',
    'userCount': '{n} Benutzer',
    'statusPending': 'Ausstehend', 'statusRemoving': 'Wird entfernt',
    'statusDisabled': 'Deaktiviert', 'statusOnline': 'Online', 'statusOffline': 'Offline',
    'deviceTitle': 'Gerät',
    'waitingApproval': 'Wartet auf Genehmigung',
    'pairingCode': 'Kopplungscode', 'acceptDevice': 'Gerät akzeptieren',
    'managedUsers': 'Verwaltete Benutzer', 'noUsersYet': 'Noch keine Benutzer erkannt.',
    'renameDevice': 'Gerät umbenennen',
    'deleteDeviceTitle': 'Gerät löschen?',
    'deleteDeviceBody': 'Das Gerät wird beim nächsten Verbinden getrennt.',
    'forceRemoveTitle': 'Zwangsweise entfernen?',
    'forceRemoveBody': 'Der Gerätedatensatz wird sofort dauerhaft vom Server gelöscht.',
    'forceRemoveMenu': 'Zwangsweise entfernen',
    'userLinked': 'Benutzer verknüpft', 'userUnlinked': 'Benutzer getrennt',
    'deviceAccepted': 'Gerät akzeptiert', 'deletionCancelled': 'Löschung abgebrochen',
    'unmanaged': 'Nicht verwaltet', 'undoDeletion': 'Löschen rückgängig',
    'copiedToClipboard': 'In Zwischenablage kopiert',
    'lastSeen': 'Zuletzt gesehen', 'timezone': 'Zeitzone',
    'agentVersion': 'Version', 'machineId': 'Geräte-ID', 'copyId': 'ID kopieren',
    'statusPendingApproval': 'Genehmigung ausstehend', 'statusPendingRemoval': 'Entfernung ausstehend',
    'recentLogs': 'Aktuelle Protokolle', 'loadLogs': 'Protokolle laden', 'refreshLogs': 'Aktualisieren',
    'agentOfflineLogs': 'Agent ist offline — verbinden Sie ihn, um Protokolle zu laden',
    'logsEmpty': 'Keine Protokollzeilen zurückgegeben',
    'never': 'nie', 'justNow': 'gerade eben',
    'minutesAgo': 'vor {n} Min.', 'hoursAgo': 'vor {n} Std.', 'daysAgo': 'vor {n} Tagen',
    'day0': 'Mo', 'day1': 'Di', 'day2': 'Mi', 'day3': 'Do',
    'day4': 'Fr', 'day5': 'Sa', 'day6': 'So',
    'dayFull0': 'Montag', 'dayFull1': 'Dienstag', 'dayFull2': 'Mittwoch',
    'dayFull3': 'Donnerstag', 'dayFull4': 'Freitag', 'dayFull5': 'Samstag', 'dayFull6': 'Sonntag',
    'dayLetter0': 'M', 'dayLetter1': 'D', 'dayLetter2': 'M', 'dayLetter3': 'D',
    'dayLetter4': 'F', 'dayLetter5': 'S', 'dayLetter6': 'S',
    'settingsTitle': 'Einstellungen', 'appearance': 'Erscheinungsbild', 'theme': 'Thema',
    'themeSystem': 'System', 'themeLight': 'Hell', 'themeDark': 'Dunkel',
    'language': 'Sprache', 'appLanguage': 'App-Sprache',
    'languageAuto': 'System (auto)', 'account': 'Konto',
    'about': 'Über', 'aboutTitle': 'Über ScreenGuard',
    'appDescription': 'Kindersicherungssoftware zur Verwaltung von Bildschirmzeit und Gerätenutzung.',
    'author': 'Autor', 'version': 'Version',
    'sourceCode': 'Quellcode', 'licenses': 'Lizenzen',
  };

  static const _pt = <String, String>{
    'profiles': 'Perfis', 'devices': 'Dispositivos', 'settings': 'Configurações',
    'cancel': 'Cancelar', 'save': 'Salvar', 'confirm': 'Confirmar',
    'retry': 'Tentar novamente', 'signOut': 'Sair', 'changeServer': 'Mudar servidor',
    'create': 'Criar', 'send': 'Enviar', 'connect': 'Conectar',
    'rename': 'Renomear', 'delete': 'Excluir', 'failedToLoad': 'Falha ao carregar',
    'now': 'Agora',
    'connectToServer': 'Conectar ao servidor',
    'findServerDesc': 'Encontre seu servidor ScreenGuard na rede local.',
    'nearbyServers': 'Servidores próximos', 'scan': 'Escanear',
    'noServersFound': 'Nenhum servidor encontrado nesta rede.',
    'manualEntry': 'Entrada manual', 'serverAddress': 'Endereço do servidor',
    'serverAddressHint': '192.168.1.10:8080 ou https://host',
    'connectManually': 'Conectar manualmente',
    'cannotReachAt': 'Não é possível alcançar o servidor em {url}',
    'cannotReachAddr': 'Não é possível alcançar o servidor. Verifique o endereço e a porta.',
    'signIn': 'Entrar', 'createAdmin': 'Criar conta de administrador',
    'createAdminDesc': 'Crie credenciais de administrador para o seu servidor.',
    'signInDesc': 'Entre para gerenciar o controle parental.',
    'username': 'Usuário', 'password': 'Senha',
    'createAccount': 'Criar conta',
    'connectionError': 'Erro de conexão. Tente novamente.',
    'newProfile': 'Novo perfil', 'profileNameLabel': 'Nome',
    'noProfilesYet': 'Sem perfis. Toque em + para criar um.',
    'locked': 'Bloqueado', 'profileTitle': 'Perfil',
    'devicesWaiting': '{n} dispositivo(s) aguardando aprovação',
    'devicesOnline': '{a}/{b} dispositivos online',
    'usedNoLimit': '{u} usado · sem limite',
    'usedWithLimit': '{u} usado · {r} restante de {l}',
    'today': 'Hoje', 'noLimit': 'Sem limite',
    'adjust': 'Ajustar', 'reset': 'Redefinir', 'apply': 'Aplicar',
    'lockNow': 'Bloquear agora',
    'lockNowConfirmTitle': 'Bloquear agora?',
    'lockNowConfirmBody': 'Isso zerará o tempo de tela restante de hoje.',
    'message': 'Mensagem', 'sendMessage': 'Enviar mensagem',
    'messageToUser': 'Mensagem para o usuário…',
    'messageSent': 'Mensagem enviada', 'timeAdjusted': 'Tempo ajustado',
    'screenTimeLocked': 'Tempo de tela bloqueado',
    'usedOf': '{u} usado', 'usedOfLimit': '{u} usado de {l}',
    'timeAdjustmentLabel': 'ajuste {a}',
    'limitsSaved': 'Limites salvos', 'scheduleSaved': 'Agenda salva',
    'languageSaved': 'Idioma salvo',
    'notifLanguage': 'Idioma das notificações',
    'linkedAccounts': 'Contas vinculadas',
    'renameProfile': 'Renomear perfil',
    'deleteProfileTitle': 'Excluir perfil?',
    'deleteProfileBody': 'Isso excluirá permanentemente o perfil e todas as suas configurações.',
    'dailyLimits': 'Limites diários',
    'dailyLimitsDesc': 'Tempo máximo de tela por dia. Deixe em branco para sem limite.',
    'saveLimits': 'Salvar limites',
    'noLimitShort': 'Sem limite', 'blocked': 'Bloqueado',
    'block': 'Bloquear', 'clear': 'Limpar', 'addLimit': 'Adicionar limite',
    'timeWindows': 'Janelas de tempo permitidas',
    'timeWindowsDesc': 'O tempo de tela só é permitido durante essas janelas.',
    'noWindows': 'Sem janelas definidas — todas as horas permitidas.',
    'addWindow': 'Adicionar janela', 'saveWindows': 'Salvar janelas',
    'addTimeWindow': 'Adicionar janela de tempo', 'editTimeWindow': 'Editar janela de tempo',
    'dayLabel': 'Dia', 'start': 'Início', 'end': 'Fim',
    'chartUsage': 'Uso',
    'noDevicesYet': 'Nenhum dispositivo registrado',
    'noDevicesDesc': 'Instale o agente em um computador gerenciado para começar.',
    'userCount': '{n} usuário(s)',
    'statusPending': 'Pendente', 'statusRemoving': 'Removendo',
    'statusDisabled': 'Desativado', 'statusOnline': 'Online', 'statusOffline': 'Offline',
    'deviceTitle': 'Dispositivo',
    'waitingApproval': 'Aguardando aprovação',
    'pairingCode': 'Código de emparelhamento', 'acceptDevice': 'Aceitar dispositivo',
    'managedUsers': 'Usuários gerenciados', 'noUsersYet': 'Nenhum usuário detectado ainda.',
    'renameDevice': 'Renomear dispositivo',
    'deleteDeviceTitle': 'Excluir dispositivo?',
    'deleteDeviceBody': 'O dispositivo será desvinculado na próxima vez que se conectar.',
    'forceRemoveTitle': 'Remover forçadamente?',
    'forceRemoveBody': 'O registro do dispositivo será excluído permanentemente do servidor imediatamente.',
    'forceRemoveMenu': 'Remover forçadamente',
    'userLinked': 'Usuário vinculado', 'userUnlinked': 'Usuário desvinculado',
    'deviceAccepted': 'Dispositivo aceito', 'deletionCancelled': 'Exclusão cancelada',
    'unmanaged': 'Não gerenciado', 'undoDeletion': 'Desfazer exclusão',
    'copiedToClipboard': 'Copiado para a área de transferência',
    'lastSeen': 'Última vez visto', 'timezone': 'Fuso horário',
    'agentVersion': 'Versão', 'machineId': 'ID da máquina', 'copyId': 'Copiar ID',
    'statusPendingApproval': 'Aprovação pendente', 'statusPendingRemoval': 'Remoção pendente',
    'recentLogs': 'Registos recentes', 'loadLogs': 'Carregar registos', 'refreshLogs': 'Atualizar',
    'agentOfflineLogs': 'O agente está offline — conecte-o para carregar registos',
    'logsEmpty': 'Nenhuma linha de registo devolvida',
    'never': 'nunca', 'justNow': 'agora mesmo',
    'minutesAgo': '{n} min atrás', 'hoursAgo': '{n} h atrás', 'daysAgo': '{n} d atrás',
    'day0': 'Seg', 'day1': 'Ter', 'day2': 'Qua', 'day3': 'Qui',
    'day4': 'Sex', 'day5': 'Sáb', 'day6': 'Dom',
    'dayFull0': 'Segunda', 'dayFull1': 'Terça', 'dayFull2': 'Quarta',
    'dayFull3': 'Quinta', 'dayFull4': 'Sexta', 'dayFull5': 'Sábado', 'dayFull6': 'Domingo',
    'dayLetter0': 'S', 'dayLetter1': 'T', 'dayLetter2': 'Q', 'dayLetter3': 'Q',
    'dayLetter4': 'S', 'dayLetter5': 'S', 'dayLetter6': 'D',
    'settingsTitle': 'Configurações', 'appearance': 'Aparência', 'theme': 'Tema',
    'themeSystem': 'Sistema', 'themeLight': 'Claro', 'themeDark': 'Escuro',
    'language': 'Idioma', 'appLanguage': 'Idioma do app',
    'languageAuto': 'Sistema (auto)', 'account': 'Conta',
    'about': 'Sobre', 'aboutTitle': 'Sobre o ScreenGuard',
    'appDescription': 'Software de controle parental para gerenciar o tempo de tela e o uso de dispositivos.',
    'author': 'Autor', 'version': 'Versão',
    'sourceCode': 'Código-fonte', 'licenses': 'Licenças',
  };

  static const _strings = <String, Map<String, String>>{
    'en': _en, 'pl': _pl, 'es': _es, 'fr': _fr, 'de': _de, 'pt': _pt,
  };
}

class _AppLocalizationsDelegate extends LocalizationsDelegate<AppLocalizations> {
  const _AppLocalizationsDelegate();

  @override
  bool isSupported(Locale locale) =>
      AppLocalizations.supportedLocales.any((l) => l.languageCode == locale.languageCode);

  @override
  Future<AppLocalizations> load(Locale locale) =>
      Future.value(AppLocalizations(locale));

  @override
  bool shouldReload(_AppLocalizationsDelegate old) => false;
}
