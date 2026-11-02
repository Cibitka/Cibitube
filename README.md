<p align="center">
  <img src="assets/banner.jpg" alt="Cibitube Banner" width="100%">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Language-Rust_2021-orange.svg?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/GUI-egui_%2F_eframe_0.30-10b981.svg?style=for-the-badge" alt="egui">
  <img src="https://img.shields.io/badge/Backend-MPV_%2B_yt--dlp-e11d48.svg?style=for-the-badge" alt="MPV">
  <img src="https://img.shields.io/badge/Platform-Linux_(Wayland_%26_X11)-3b82f6.svg?style=for-the-badge&logo=linux" alt="Linux">
  <img src="https://img.shields.io/github/v/release/Cibitka/Cibitube?style=for-the-badge&color=8b5cf6" alt="Release">
  <img src="https://img.shields.io/badge/License-MIT-gray.svg?style=for-the-badge" alt="License">
</p>

<h3 align="center">
  ⚡ Сверхбыстрый, стильный и легковесный клиент YouTube Music для Linux на Rust
</h3>

<p align="center">
  <em>Вдохновлено лаконичным мобильным дизайном Metrolist, но переработано специально для мощного настольного ПК.</em>
</p>

---

## 🌟 Ключевые возможности

- 🚀 **Мгновенный запуск и низкое потребление ресурсов**: Потребляет всего ~30–50 МБ оперативной памяти и практически 0% нагрузки на процессор в фоновом режиме.
- 🎨 **Кастомизация внешнего вида**:
  - AMOLED True Black, Dark Slate и светлая темы.
  - 5 предустановленных акцентных цветов + полноценный выбор любого собственного RGB-цвета через палитру.
  - Анимированный волнистый ползунок («змейка») или строгий классический слайдер.
  - HD-апскейл обложек (до 800×800) без потери четкости.
- 👤 **Авторизация и профили YouTube Music**:
  - Автоматический импорт cookies из Firefox без ручного ввода.
  - Поддержка нескольких связанных аккаунтов Google и быстрое переключение между ними в 1 клик.
  - Полная синхронизация пользовательских плейлистов и «Понравившихся» треков.
- 📜 **Синхронизированные тексты песен (Synced Lyrics)**:
  - Автоматическая подгрузка текста песни из базы LRCLIB.
  - Караоке-подсветка активной строки с плавной автопрокруткой.
  - Возможность замены обложки на текст прямо в окне воспроизведения.
  - Перемотка трека по клику на любую строчку текста.
- 🔊 **Полноэкранный плеер (Now Playing)**:
  - Динамическая адаптация размера обложки под размер окна.
  - Встроенный ползунок регулировки громкости и кнопка быстрого отключения звука (Mute).
  - Перемешивание всей очереди (без сброса текущего играющего трека) и режимы повтора (все/один).
- ⬇ **Скачивание музыки**:
  - Скачивание отдельных треков или целых плейлистов в высоком качестве (320 kbps Opus) в один клик.
  - Библиотека оффлайн-музыки с возможностью локального воспроизведения.
- 🎮 **Интеграция с Discord Rich Presence**:
  - Красивое отображение играющего трека, исполнителя, обложки и кликабельных кнопок («Слушать в Cibitube»).
- 🔄 **Встроенный авто-апдейтер**:
  - Проверка обновлений прямо в приложении через GitHub Releases.
  - Скачивание, проверка и установка новой версии в один клик с автоматической заменой старого бинарника.
- 🖥 **Стабильность в Wayland / Hyprland**:
  - Встроенный независимый фоновый цикл предотвращает засыпание сокета и предупреждения «Приложение не отвечает» при переключении виртуальных рабочих столов.

---

## 📦 Установка

### 1. Системные зависимости

Для воспроизведения звука и работы с потоками требуются `mpv` и `yt-dlp`:

**Arch Linux / Manjaro / EndeavourOS:**
```bash
sudo pacman -S mpv yt-dlp
```

**Ubuntu / Debian / Linux Mint:**
```bash
sudo apt update && sudo apt install mpv yt-dlp
```

**Fedora:**
```bash
sudo dnf install mpv yt-dlp
```

---

### 2. Быстрая установка готового релиза

Скачайте скомпилированный бинарный файл со страницы [Releases](https://github.com/Cibitka/Cibitube/releases/latest) или выполните команду в терминале:

```bash
# Создаем папки при необходимости
mkdir -p ~/.local/bin ~/.local/share/applications ~/.local/share/icons/hicolor/512x512/apps

# Скачиваем бинарный файл последней версии
curl -sL https://github.com/Cibitka/Cibitube/releases/latest/download/cibitube -o ~/.local/bin/cibitube
chmod +x ~/.local/bin/cibitube

# Скачиваем официальную иконку
curl -sL https://raw.githubusercontent.com/Cibitka/Cibitube/main/assets/icon.png -o ~/.local/share/icons/hicolor/512x512/apps/cibitube.png

# Создаем ярлык приложения в системном меню
cat <<EOF > ~/.local/share/applications/cibitube.desktop
[Desktop Entry]
Name=Cibitube
GenericName=YouTube Music Client
Comment=Быстрый и удобный плеер YouTube Music на Rust
Exec=$HOME/.local/bin/cibitube
Icon=cibitube
Terminal=false
Type=Application
Categories=AudioVideo;Audio;Player;Music;
StartupWMClass=cibitube
Keywords=music;youtube;audio;streaming;cibitube;
EOF

update-desktop-database ~/.local/share/applications || true
```

Теперь **Cibitube** появится в вашем меню приложений (Rofi, Wofi, GNOME, KDE, App Launcher)!

---

### 3. Сборка из исходников

Если вы хотите собрать приложение самостоятельно:

1. Установите компилятор **Rust** (через [rustup.rs](https://rustup.rs/)):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Склонируйте репозиторий и соберите оптимизированную версию:
   ```bash
   git clone https://github.com/Cibitka/Cibitube.git
   cd Cibitube
   cargo build --release
   ```

3. Установите бинарник в систему:
   ```bash
   cp target/release/metrolist-desktop ~/.local/bin/cibitube
   chmod +x ~/.local/bin/cibitube
   ```

---

## ⌨ Управление и горячие клавиши

| Клавиша / Действие | Функция |
|---|---|
| `Пробел` | Пауза / Возобновление воспроизведения |
| `Alt + →` / `⏭` | Следующий трек |
| `Alt + ←` / `⏮` | Предыдущий трек |
| `Клик по строке текста` | Мгновенная перемотка трека к выбранной фразе |
| `Клик на «♫ Текст песни»` | Переключение между обложкой альбома и текстом |
| `🔀 Перемешать` | Перемешать очередь треков (текущий трек сохраняется) |
| `⬇ Скачать` | Загрузить трек или плейлист на диск для оффлайн-прослушивания |

---

## 🛠 Настройки

Все параметры сохраняются в `~/.local/share/metrolist/settings.json` и доступны через встроенное окно настроек `⚙ Настройки`:
- **Аккаунты**: Вход в Google-аккаунт, выбор активного профиля YouTube Music.
- **Оформление**: Цвета акцента, стили ползунка, темы оформления.
- **Плеер**: Стили фона (чистый черный, сплошной, градиент), скругление обложек альбомов.
- **Текст песни**: Центрирование текста, автоскролл, синхронизация.
- **Звук**: Выбор битрейта (до 320 kbps Opus), нормализация громкости, пропуск тишины, SponsorBlock.
- **Discord**: Включение/выключение статуса "Слушает в Discord".
- **Обновления**: Проверка и авто-установка обновлений прямо из интерфейса.

---

## 🤝 Вклад в проект и благодарности

- Идея и визуальная эстетика вдохновлены проектом [Metrolist](https://github.com/2096-764/metrolist).
- База синхронизированных текстов: [LRCLIB](https://lrclib.net/).
- Графический интерфейс: библиотека [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe).
- Воспроизведение аудио: проект [mpv](https://mpv.io/) и [yt-dlp](https://github.com/yt-dlp/yt-dlp).

---

## 📄 Лицензия

Проект распространяется под открытой лицензией [MIT](LICENSE).
