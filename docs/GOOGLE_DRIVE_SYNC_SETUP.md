# Google Drive sync / Синхронизация через Google Диск

[English](#english) | [Русский](#русский)

---

## English

### How it works
Cibitube writes a small file `cibitube_sync_state.json` into a folder you choose. If that folder is synced with your Google Drive, another computer running Cibitube can pick up your current track, position and queue.

Cibitube does **not** ask for access to your Google account and never sees your Google password (signing in is done by Google Drive for Desktop or by `rclone`, not by Cibitube).

> Phone sync will arrive together with the Android client. For now sync works between computers running Cibitube.

### Step 1. Create the folder
In Google Drive create a folder named `Cibitube-Sync`.

### Step 2. Make the folder available on your computer
- **Windows / macOS:** install [Google Drive for Desktop](https://www.google.com/drive/download/). The folder appears as `My Drive/Cibitube-Sync`.
- **Linux (rclone):**
  ```bash
  sudo pacman -S rclone fuse3        # Arch; on Debian/Ubuntu: sudo apt install rclone fuse3
  rclone config                      # n (new remote) -> name: gdrive -> storage: drive -> finish, sign in in the browser
  mkdir -p ~/GoogleDrive/Cibitube-Sync
  rclone mount gdrive:Cibitube-Sync ~/GoogleDrive/Cibitube-Sync --vfs-cache-mode writes --daemon
  ```
  To unmount: `fusermount3 -u ~/GoogleDrive/Cibitube-Sync`.

### Step 3. Point Cibitube to the folder
Open **Settings → Cross-device sync**, press **Choose sync folder** and select the folder from step 2. Repeat on every computer.

### Step 4. Use it
- **Save position to Google Drive** writes the current track, position and queue.
- **Resume from another device** reads them and continues playback.

### File format
```json
{
  "trackId": "VIDEO_ID",
  "title": "Example Track",
  "artist": "Example Artist",
  "album": null,
  "positionSecs": 47.5,
  "durationSecs": 196.0,
  "isPlaying": false,
  "playbackSpeed": 1.0,
  "sleepTimerMins": null,
  "queue": [],
  "queueIndex": 0,
  "repeatMode": "all",
  "shuffleEnabled": false,
  "sourceDevice": "desktop",
  "updatedAt": "2026-01-01T12:00:00Z"
}
```

### Troubleshooting
- *"Sync folder is not writable"*: check that the mount is running (`mount | grep Cibitube-Sync`) and that you have write permission.
- *The other computer does not see changes*: Google Drive needs a few seconds to sync; with `rclone mount` keep `--vfs-cache-mode writes`.
- *Wrong position after resume*: both computers should have correct clocks (NTP).

---

## Русский

### Как это работает
Cibitube записывает маленький файл `cibitube_sync_state.json` в выбранную вами папку. Если эта папка синхронизируется с Google Диском, другой компьютер с Cibitube подхватит текущий трек, позицию и очередь.

Cibitube **не запрашивает** доступ к вашему Google-аккаунту и не видит ваш пароль: вход выполняют Google Диск для ПК или `rclone`, а не Cibitube.

> Синхронизация с телефоном появится вместе с Android-клиентом. Сейчас она работает между компьютерами с Cibitube.

### Шаг 1. Создайте папку
В Google Диске создайте папку `Cibitube-Sync`.

### Шаг 2. Сделайте папку доступной на компьютере
- **Windows / macOS:** установите [Google Диск для ПК](https://www.google.com/intl/ru/drive/download/). Папка появится как `Мой диск/Cibitube-Sync`.
- **Linux (rclone):**
  ```bash
  sudo pacman -S rclone fuse3        # Arch; в Debian/Ubuntu: sudo apt install rclone fuse3
  rclone config                      # n (новый remote) -> имя: gdrive -> тип: drive -> завершить, войти в браузере
  mkdir -p ~/GoogleDrive/Cibitube-Sync
  rclone mount gdrive:Cibitube-Sync ~/GoogleDrive/Cibitube-Sync --vfs-cache-mode writes --daemon
  ```
  Отмонтировать: `fusermount3 -u ~/GoogleDrive/Cibitube-Sync`.

### Шаг 3. Укажите папку в Cibitube
Откройте **Настройки → Синхронизация между устройствами**, нажмите **Выбрать папку синхронизации** и укажите папку из шага 2. Повторите на каждом компьютере.

### Шаг 4. Пользуйтесь
- **Сохранить позицию в Google Drive** записывает текущий трек, позицию и очередь.
- **Продолжить с другого устройства** читает их и продолжает воспроизведение.

### Формат файла
Пример см. в английском разделе выше (поля `trackId`, `positionSecs`, `updatedAt` и другие).

### Если что-то не работает
- *«Папка синхронизации недоступна для записи»*: проверьте, что монтирование запущено (`mount | grep Cibitube-Sync`) и у вас есть права на запись.
- *Другой компьютер не видит изменений*: Google Диску нужно несколько секунд; при `rclone mount` оставляйте `--vfs-cache-mode writes`.
- *Позиция после продолжения неверна*: на обоих компьютерах должны быть правильные часы (NTP).
