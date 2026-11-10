// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Russian (`ru`).
class L10nRu extends L10n {
  L10nRu([String locale = 'ru']) : super(locale);

  @override
  String get onboardTitle => 'Как назвать это устройство?';

  @override
  String get onboardHint => 'Это имя увидят только ваши устройства.';

  @override
  String get nameLabel => 'Имя устройства';

  @override
  String get continueAction => 'Продолжить';

  @override
  String get myDevices => 'Мои устройства';

  @override
  String get addDevice => 'Добавить устройство';

  @override
  String get add => 'Добавить';

  @override
  String get history => 'История';

  @override
  String get settings => 'Настройки';

  @override
  String get devices => 'Устройства';

  @override
  String get nearby => 'Рядом';

  @override
  String get offline => 'Не в сети';

  @override
  String get homeHint => 'Выберите устройство, чтобы отправить файлы';

  @override
  String get emptyTitle => 'Пока здесь только это устройство';

  @override
  String get emptyBody =>
      'Установите Zerohop на другое устройство и привяжите его один раз. Дальше файлы пойдут напрямую, без интернета.';

  @override
  String get dropTitle => 'Перетащите файлы сюда';

  @override
  String dropHint(String name) {
    return 'Файлы уйдут напрямую на $name, без интернета';
  }

  @override
  String dropHintOffline(String name) {
    return '$name сейчас не в этой сети';
  }

  @override
  String get chooseFiles => 'Выбрать файлы';

  @override
  String get chooseFolder => 'Выбрать папку';

  @override
  String get addFiles => 'Добавить файлы';

  @override
  String get clear => 'Очистить';

  @override
  String get send => 'Отправить';

  @override
  String sendTo(String name) {
    return 'Отправить на $name';
  }

  @override
  String removeItem(String name) {
    return 'Убрать $name';
  }

  @override
  String get folderKind => 'Папка';

  @override
  String fileCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count файла',
      many: '$count файлов',
      few: '$count файла',
      one: '$count файл',
    );
    return '$_temp0';
  }

  @override
  String sentCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count файла отправлено',
      many: '$count файлов отправлено',
      few: '$count файла отправлено',
      one: '$count файл отправлен',
    );
    return '$_temp0';
  }

  @override
  String receivedCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count файла получено',
      many: '$count файлов получено',
      few: '$count файла получено',
      one: '$count файл получен',
    );
    return '$_temp0';
  }

  @override
  String sizeSummary(String count, String size) {
    return '$count, $size';
  }

  @override
  String get cancelTransfer => 'Отменить';

  @override
  String get done => 'Готово';

  @override
  String doneSent(String summary, String name) {
    return '$summary на $name. Все файлы проверены.';
  }

  @override
  String doneReceived(String summary, String name) {
    return '$summary с $name. Все файлы проверены.';
  }

  @override
  String heldNote(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other:
          '$count объекта похожи на программы и ждут вашего решения в истории.',
      many:
          '$count объектов похожи на программы и ждут вашего решения в истории.',
      few:
          '$count объекта похожи на программы и ждут вашего решения в истории.',
      one: '$count объект похож на программу и ждёт вашего решения в истории.',
    );
    return '$_temp0';
  }

  @override
  String heldNoteSent(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other:
          '$count объекта похожи на программы и ждут решения на другом устройстве.',
      many:
          '$count объектов похожи на программы и ждут решения на другом устройстве.',
      few:
          '$count объекта похожи на программы и ждут решения на другом устройстве.',
      one:
          '$count объект похож на программу и ждёт решения на другом устройстве.',
    );
    return '$_temp0';
  }

  @override
  String get sendMore => 'Отправить ещё';

  @override
  String get close => 'Закрыть';

  @override
  String get resume => 'Продолжить';

  @override
  String get showInFolder => 'Показать в папке';

  @override
  String progressOf(String done, String total) {
    return '$done из $total';
  }

  @override
  String perSecond(String size) {
    return '$size/с';
  }

  @override
  String sendingTo(String name) {
    return 'Отправляем на $name';
  }

  @override
  String receivingFrom(String name) {
    return 'Получаем с $name';
  }

  @override
  String get waitingAnswer => 'Ждём ответа на другом устройстве';

  @override
  String get encryptedLine => 'Напрямую по локальной сети, с шифрованием';

  @override
  String incomingTitle(String name) {
    return '$name отправляет файлы';
  }

  @override
  String get decline => 'Отклонить';

  @override
  String get accept => 'Принять';

  @override
  String programWarning(String name) {
    return 'Среди файлов есть программа: $name.';
  }

  @override
  String get programAdvice => 'Принимайте, только если отправили её сами.';

  @override
  String get receiveFrom => 'Приём с этого устройства';

  @override
  String get receiving => 'Приём файлов';

  @override
  String get modeAsk => 'Всегда спрашивать';

  @override
  String get modeAskSub => 'Каждая передача ждёт вашего ответа';

  @override
  String get modeSmart => 'Автоматически, кроме программ';

  @override
  String get modeSmartSub => 'Спросим, только если среди файлов есть программы';

  @override
  String get modeAuto => 'Автоматически';

  @override
  String get modeAutoSub => 'Всё сохраняется без вопросов';

  @override
  String get modeRefuse => 'Не принимать';

  @override
  String get modeRefuseSub => 'Это устройство не сможет отправлять файлы сюда';

  @override
  String get modeDefault => 'Как в настройках';

  @override
  String deviceCode(String code) {
    return 'Код устройства $code';
  }

  @override
  String get removeDevice => 'Отключить устройство';

  @override
  String removeTitle(String name) {
    return 'Отключить $name?';
  }

  @override
  String get removeBody =>
      'Устройство больше не сможет обмениваться файлами с остальными. То, что оно уже получило, останется на нём.';

  @override
  String get removeAction => 'Отключить';

  @override
  String get cancelAction => 'Отмена';

  @override
  String get back => 'Назад';

  @override
  String get pairIntro =>
      'Откройте Zerohop на другом устройстве. Достаточно привязать его один раз, и оно увидит все ваши устройства.';

  @override
  String get showCode => 'Показать код здесь';

  @override
  String get showCodeSub => 'Шесть цифр для ввода на другом устройстве';

  @override
  String get enterCode => 'Ввести код';

  @override
  String get enterCodeSub => 'Если он уже показан на другом устройстве';

  @override
  String get codeTitle => 'Код для привязки';

  @override
  String roomLine(String room, int seconds) {
    return 'Комната $room. Код работает один раз, осталось $seconds с.';
  }

  @override
  String get waitingForDevice => 'Ждём другое устройство';

  @override
  String get enterTitle => 'Введите код';

  @override
  String get enterLabel => 'Шесть цифр с экрана другого устройства';

  @override
  String get roomsTitle => 'Найдено в этой сети';

  @override
  String get roomsSearching => 'Ищем устройство с открытым кодом';

  @override
  String room(String label) {
    return 'Комната $label';
  }

  @override
  String get manualAddress => 'Указать адрес вручную';

  @override
  String get addressLabel => 'Адрес, например 192.168.1.20:53319';

  @override
  String get connecting => 'Подключаемся';

  @override
  String get compareTitle => 'Проверьте число';

  @override
  String get compareSub => 'Оно должно быть одинаковым на обоих экранах';

  @override
  String get otherDevice => 'Другое устройство';

  @override
  String get numbersMatch => 'Числа совпадают';

  @override
  String get numbersDiffer => 'Не совпадают';

  @override
  String get finishing => 'Завершаем привязку';

  @override
  String pairedToast(String name) {
    return '$name: устройство добавлено';
  }

  @override
  String get historyEmpty => 'Здесь появятся отправленные и полученные файлы.';

  @override
  String toName(String name, String size) {
    return 'На $name, $size';
  }

  @override
  String fromName(String name, String size) {
    return 'С $name, $size';
  }

  @override
  String get historyDeclined => 'Передача отклонена';

  @override
  String get historyCancelled => 'Передача отменена';

  @override
  String get historyInterrupted => 'Передача прервана';

  @override
  String get heldSection => 'Ждут вашего решения';

  @override
  String get heldBody =>
      'Эти файлы выдают себя за другие, но внутри похожи на программы. Если ничего не делать, через сутки они удалятся.';

  @override
  String get letIn => 'Оставить';

  @override
  String get delete => 'Удалить';

  @override
  String heldFrom(String name, String size) {
    return 'С $name, $size';
  }

  @override
  String get unfinishedSection => 'Не завершено';

  @override
  String get forget => 'Забыть';

  @override
  String get clearHistory => 'Очистить историю';

  @override
  String get visible => 'Видно в сети';

  @override
  String get visibleSub => 'Ваши устройства находят это устройство';

  @override
  String get folder => 'Папка для файлов';

  @override
  String get downloadsFolder => 'Загрузки';

  @override
  String get filesAppFolder => 'Файлы › Zerohop';

  @override
  String get change => 'Изменить';

  @override
  String get open => 'Открыть';

  @override
  String get confirmNew => 'Подтверждать новые устройства';

  @override
  String get confirmNewSub =>
      'Устройство, добавленное через другое, будет ждать вашего разрешения здесь';

  @override
  String get color => 'Цвет';

  @override
  String get cobalt => 'Кобальт';

  @override
  String get sunset => 'Закат';

  @override
  String get mint => 'Мята';

  @override
  String get appearance => 'Оформление';

  @override
  String get themeSystem => 'Как в системе';

  @override
  String get themeLight => 'Светлая';

  @override
  String get themeDark => 'Тёмная';

  @override
  String get language => 'Язык';

  @override
  String get langSystem => 'Как в системе';

  @override
  String get thisDevice => 'Это устройство';

  @override
  String get startOver => 'Начать заново';

  @override
  String get startOverTitle => 'Начать заново на этом устройстве?';

  @override
  String get startOverBody =>
      'Устройство отключится от остальных и забудет их. Полученные файлы останутся на месте.';

  @override
  String get removedBanner => 'Это устройство отключили от остальных.';

  @override
  String get folderProblem =>
      'Выбранная папка недоступна, поэтому файлы сохраняются в папку по умолчанию.';

  @override
  String newDeviceTitle(String name) {
    return 'Новое устройство: $name';
  }

  @override
  String get newDeviceBody =>
      'Его добавили через другое ваше устройство. Разрешить ему отправлять файлы сюда?';

  @override
  String get allow => 'Разрешить';

  @override
  String get errUnreachable =>
      'Устройство недоступно. Проверьте, что Zerohop открыт и оба устройства в одной сети.';

  @override
  String get errConnectionLost =>
      'Соединение прервалось. Передачу можно продолжить.';

  @override
  String get errDeclined => 'Передачу отклонили на другом устройстве.';

  @override
  String get errNoSpace => 'На другом устройстве не хватает места.';

  @override
  String get errBusy => 'Другое устройство занято. Попробуйте через минуту.';

  @override
  String get errInvalidName =>
      'Одно из имён нельзя сохранить на другом устройстве.';

  @override
  String get errCancelled => 'Передача отменена.';

  @override
  String get errCancelledByPeer => 'Передачу отменили на другом устройстве.';

  @override
  String get errIntegrity => 'Файл повредился по пути. Отправьте его ещё раз.';

  @override
  String get errSourceChanged =>
      'Файл изменился во время отправки. Отправьте его ещё раз.';

  @override
  String get errSourceUnreadable => 'Не удалось прочитать файл.';

  @override
  String get errNothingToSend => 'Здесь нечего отправлять.';

  @override
  String get errRemoved => 'Это устройство отключили от остальных.';

  @override
  String get errPeerRemoved => 'Это устройство больше не среди ваших.';

  @override
  String get errCannotResume =>
      'Продолжить уже нельзя. Отправьте файлы заново.';

  @override
  String get errProtocol =>
      'Другое устройство ответило неверно. Обновите Zerohop на обоих.';

  @override
  String get errStorage => 'Не удалось записать файлы на диск.';

  @override
  String get errPeerStorage => 'Другое устройство не смогло сохранить файлы.';

  @override
  String get errAwaiting => 'Сначала подтвердите это устройство.';

  @override
  String get errInUse => 'Zerohop уже открыт.';

  @override
  String get errWrongCode =>
      'Код не подошёл. Покажите новый код и попробуйте снова.';

  @override
  String get errTimeout => 'Время вышло. Покажите новый код.';

  @override
  String get errPairDeclined => 'Привязку отклонили.';

  @override
  String get errPairUnreachable =>
      'Не удалось подключиться. Проверьте, что оба устройства в одной сети.';

  @override
  String get errPairFailed => 'Привязать не удалось. Попробуйте ещё раз.';

  @override
  String get errAlreadyLinked =>
      'Это устройство уже связано с другими. Покажите код здесь, а введите его на новом устройстве.';

  @override
  String get errBadAddress => 'Адрес выглядит неверно.';

  @override
  String get errBadName => 'Введите имя покороче.';

  @override
  String get errFolder => 'Эту папку использовать нельзя.';

  @override
  String get errGeneric => 'Что-то пошло не так. Попробуйте ещё раз.';

  @override
  String get unitB => 'Б';

  @override
  String get unitKB => 'КБ';

  @override
  String get unitMB => 'МБ';

  @override
  String get unitGB => 'ГБ';

  @override
  String get unitTB => 'ТБ';

  @override
  String get clipboard => 'Общий буфер';

  @override
  String get clipboardSub =>
      'Скопированное на одном устройстве можно вставить на другом. Пароли из менеджеров паролей не передаются.';

  @override
  String get clipboardOnDevice => 'Скопированное здесь вставляется там';

  @override
  String get clipboardOffDevice => 'Выключен для этого устройства';

  @override
  String get clipboardOff => 'Выключен';

  @override
  String get clipboardOneTap => 'Отправка одним нажатием';

  @override
  String get clipboardSend => 'Отправить';

  @override
  String get clipboardEnable => 'Включить';

  @override
  String get clipboardOptions => 'Настроить';

  @override
  String get clipTurnOnFirst => 'Сначала включите общий буфер в настройках.';

  @override
  String get clipSendTo => 'Отправлять на него';

  @override
  String get clipReceiveFrom => 'Принимать с него';

  @override
  String get clipText => 'Текст и ссылки';

  @override
  String get clipImages => 'Картинки';

  @override
  String get clipAllow => 'Разрешить';

  @override
  String get clipAllowTitle => 'Разрешение macOS';

  @override
  String get clipNeedsAllow =>
      'Разрешите Zerohop читать буфер: выберите «Разрешать» в настройках macOS';

  @override
  String get clipSent => 'Буфер отправлен на устройства рядом';

  @override
  String clipArrived(String name) {
    return 'Буфер получен: $name';
  }

  @override
  String get clipNoDevices =>
      'Рядом нет устройств, которым можно отправить буфер';

  @override
  String get clipFailed => 'Не удалось отправить буфер';

  @override
  String get clipEmpty => 'Сначала скопируйте текст или картинку';

  @override
  String get clipSecret => 'Это скопировано как секретное и не отправляется';

  @override
  String get clipTooLarge => 'Это слишком большое для буфера';

  @override
  String get clipSame => 'Это уже пришло с другого устройства';

  @override
  String get clipDenied => 'Система не разрешила прочитать буфер';

  @override
  String get clipTileTitle => 'Кнопка в шторке';

  @override
  String get clipTileSub =>
      'Отправляйте скопированное из шторки или из уведомления, не открывая приложение.';

  @override
  String get clipTileAdd => 'Добавить';

  @override
  String get clipTileManual =>
      'Добавьте плитку «Отправить буфер» в шторку вручную';

  @override
  String get clipMenuTitle => 'Пункт «В Zerohop»';

  @override
  String get clipMenuSub =>
      'Появится в меню выделенного текста, а текст из «Поделиться» будет сразу уходить в буфер. Вызвать его сможет и любое приложение на экране, поэтому включайте, если доверяете своим приложениям.';

  @override
  String get errKeyUnavailable =>
      'Система не дала доступ к ключу этого устройства. Разрешите доступ и повторите.';

  @override
  String get errKeyLocked =>
      'Ключ этого устройства больше нельзя открыть. Чтобы пользоваться приложением, начните заново и привяжите устройства ещё раз.';

  @override
  String get keyRetry => 'Повторить';

  @override
  String get keyStartOver => 'Начать заново';

  @override
  String get shareNoDevices => 'Сначала добавьте устройство';

  @override
  String get shareTitle => 'Куда отправить?';

  @override
  String shareMore(int count) {
    return 'и ещё $count';
  }

  @override
  String get shareChoose => 'Выберите устройство';

  @override
  String get shareAwaiting => 'Ждёт вашего разрешения';

  @override
  String get shareNobodyNearby =>
      'Сейчас рядом нет ни одного устройства. Откройте Zerohop на том, куда отправляете, и оно станет доступно.';
}
