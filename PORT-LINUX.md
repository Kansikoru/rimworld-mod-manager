# Порт на Linux (папка 3.1-linux, ветка `port/linux`)

Отдельная копия проекта для версии под Linux. Это git worktree того же репозитория: своя
папка, своя ветка `port/linux`, своя папка сборки `target/`. Основная версия в `D:\1\3`
(ветка `main`) не меняется. Когда порт будет готов, ветку сольём в `main` и выложим.

SteamOS (Steam Deck) — это тоже Linux. Её особенности делаются в `3.2-steamos` поверх этой
ветки. macOS — в `3.3-macos`.

## Структура

Всё, что различается между ОС, собрано в `src/platform/`:

| Файл | Что внутри |
|---|---|
| `platform/mod.rs` | общий интерфейс и выбор файла под ОС при сборке |
| `platform/windows.rs` | Windows: реестр Steam, `AppData\LocalLow`, `%LOCALAPPDATA%`, `tasklist`, `C:\Windows\Fonts` |
| `platform/linux.rs` | Linux: папки Steam, `~/.config/unity3d`, `~/.local/share`, `/proc`, папки шрифтов |

Каждый файл ОС даёт одно и то же:

- `UNITY_EXE`, `WAYLITH_EXE` — исполняемый файл игры 1.6 и 1.7;
- `STEAM_API_LIB`, `steam_api_in_game()` — библиотека Steam API и где её искать в папке игры;
- `steam_install_dir()` — папка клиента Steam;
- `game_data_dirs()` — папка данных игры (Config, Saves, ModLists, Player.log);
- `manager_data_base()` — где лежит папка `RimWorldModManager` самого менеджера;
- `desktop_dir()`, `game_pid()`, `system_font()`.

Остальной код спрашивает это через `game_install`, `steam`, `mods_config` и не знает про ОС.

## Пути на Linux

| Что | Где ищем |
|---|---|
| Steam | `~/.steam/steam`, `~/.steam/root`, `$XDG_DATA_HOME/Steam`, `~/.local/share/Steam`, Flatpak `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`, Snap `~/snap/steam/common/.local/share/Steam` |
| Данные игры | `$XDG_CONFIG_HOME/unity3d/Ludeon Studios/RimWorld by Ludeon Studios` (обычно `~/.config/...`), и то же внутри Flatpak Steam |
| Данные менеджера | `$XDG_DATA_HOME/RimWorldModManager` (обычно `~/.local/share/...`) |
| Игра 1.6 | `RimWorldLinux`, Steam API: `RimWorldLinux_Data/Plugins/(x86_64/)libsteam_api.so` |
| Игра 1.7 | `RimWorld` — **не проверено** |
| Запущена ли игра | имена процессов в `/proc/<pid>/comm` |

## Сборка в WSL (CachyOS)

Linux-версия собирается и запускается в WSL с CachyOS (`wsl -d cachyos`). Исходники берутся
из этой папки (`/mnt/d/1/3.1-linux`), а сборка лежит внутри Linux (`~/rwmm-target`): так
быстрее, и она не смешивается с Windows-сборкой в `target/`.

```sh
cd /mnt/d/1/3.1-linux
CARGO_TARGET_DIR=~/rwmm-target cargo build      # или test, clippy --all-targets
~/rwmm-target/debug/rimworld-mod-manager
```

Окно открывается через WSLg. В CachyOS под WSL работает X11, а Wayland — нет:
`XDG_RUNTIME_DIR` указывает на `/run/user/1000`, где нет сокета WSLg. Поэтому программу
запускать так: `env -u WAYLAND_DISPLAY ~/rwmm-target/debug/rimworld-mod-manager`. На
настоящем Linux это не нужно.

Пакеты, поставленные в CachyOS: `base-devel git rustup pkgconf mesa libxkbcommon
libxkbcommon-x11 wayland libx11 libxcursor libxrandr libxi fontconfig noto-fonts wqy-microhei
lib32-glibc lib32-gcc-libs` (последние два нужны SteamCMD).

## Сделано

- [x] Модуль `platform`, Windows-код перенесён в `platform/windows.rs` без изменений поведения.
- [x] Реализация для Linux: пути Steam, игры, менеджера, поиск процесса, системные шрифты.
- [x] `Cargo.toml`: окно под Linux через X11 и Wayland.
- [x] Реестр Windows (`winreg`) больше не используется вне `platform/windows.rs`.
- [x] **Собирается под Linux** (CachyOS в WSL): clippy чистый, 219 тестов проходят; под
      Windows по-прежнему clippy чистый и 220 тестов.
- [x] `build.rs`: на Linux подключается `libstdc++` (его требует кодек текстур `intel_tex_2`).
- [x] Ссылки SteamCMD в `Mods` удаляются правильно и на Linux (`fs_util::remove_dir_link`:
      junction на Windows, symlink на Linux). Раньше на Linux «Удалить» и замена ссылки падали.
- [x] Тесты с путями Windows (`C:\...`) идут только на Windows, для Linux есть такие же с `/home/...`.
- [x] Китайский, японский и корейский: шрифты Linux (Noto CJK, WenQuanYi, Droid Sans Fallback).
- [x] **SteamCMD для Linux**: ставится из `steamcmd_linux.tar.gz` (крейт `tar`, права на запуск
      сохраняются), запускается `steamcmd.sh` с `HOME` = своя папка, поэтому моды скачиваются в
      `<папка SteamCMD>/Steam/steamapps/workshop/content/294100`, а домашняя папка не трогается.
      Остановка убивает всю группу процессов (`steamcmd.sh` и сам SteamCMD). Без 32-битных
      библиотек программа говорит, что поставить (pacman / apt / dnf). Живой тест прошёл:
      установка, обновление, скачивание HugsLib, ссылка в `Mods`, консоль.
- [x] Проверено в окне на поддельной установке (`~/.local/share/Steam`, `~/.config/unity3d`):
      моды из Workshop и игры найдены, активный список прочитан из `ModsConfig.xml`.

## Осталось

1. **Тексты** со словом `steam_api64.dll` в `i18n.rs`, `cleaner.rs`, `app.rs`,
   `app/translations_tab.rs`: подставлять `platform::STEAM_API_LIB`.
2. **Запуск игры**: сейчас исполняемый файл запускается напрямую. На Linux проверить, нужен ли
   запуск через Steam (`steam -applaunch 294100`) или `steam_appid.txt`.
3. **Список шрифтов в Настройках** сейчас из Windows (Segoe UI, Verdana, Tahoma): на Linux их нет,
   остаётся встроенный Montserrat. Показывать шрифты, которые есть в системе.
4. **Видеопамять**: сейчас только DXGI (Windows). На Linux: `/sys/class/drm/card*/device/mem_info_vram_total`
   (AMD), `nvidia-smi` (NVIDIA).
5. **Нагрузка процесса** (время загрузки без мода-компаньона): `process_stats` на Linux через
   `/proc/<pid>/stat` и `/proc/<pid>/io`.
6. **Сравнение путей без учёта регистра** (`mods_config::save_data_folder_for` и др.): на Linux
   регистр важен.
7. **Проверить на настоящей установке RimWorld под Linux**: имя исполняемого файла 1.7 и
   папку данных 1.7.
8. **CI**: задание `ubuntu-latest` в `.github/workflows/`.
9. **Выпуск**: `tar.gz` и AppImage.
