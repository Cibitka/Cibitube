#!/usr/bin/env bash
# Проверка перед коммитом: можно ли публиковать staged-изменения в ПУБЛИЧНЫЙ репозиторий.
# Использование: git add <файлы>; ./scripts/check-public-safe.sh
# Код возврата: 0 = можно, 1 = НЕЛЬЗЯ (есть запрещённые файлы или похоже на секреты).
# Скрипт печатает только имена файлов, значения секретов не выводятся.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2

fail=0
warn=0
self=':!scripts/check-public-safe.sh'

mapfile -t files < <(git diff --cached --name-only --diff-filter=ACMR)
if [ "${#files[@]}" -eq 0 ]; then
  echo "Нет staged-файлов, проверять нечего."
  exit 0
fi

# 1. Запрещённые имена и типы файлов
BAD_NAME='(^|/)\.agents/|(^|/)(AGENTS|GEMINI)\.md$|(^|/)(start-agy|test-guard)\.py$|(^|/)(START_HERE|QUICKSTART|INSTALL_ANTIGRAVITY_CLI)\.md$|(\.jks$|\.keystore$|\.p12$|\.pfx$|\.pem$|\.key$|(^|/)\.env($|\.)|(^|/)settings\.json$|(^|/)local\.properties$|(^|/)keystore\.properties$|google-services\.json$|\.apk$|\.aab$|\.exe$|\.dll$|\.so$|\.dylib$|\.deb$|\.AppImage$|(^|/)target/|(^|/)dist/|(^|/)release-assets/|(^|/)build/|(^|/)\.gradle/|cibitube_sync_state\.json$|antigravity|push-to-github|cibitube-cli-status|(^|/)AGENT_[A-Z_]*\.md$|(^|/)CLAUDE(\.local)?\.md$|(^|/)\.claude/|MASTER_LOG|\.log$|\.bak(-|$)|\.new$|\.mp3$|\.flac$|\.opus$|\.m4a$|\.ogg$|\.wav$|\.lrc$|\.mp4$|\.mkv$|\.webm$|id_rsa|id_ed25519|\.DS_Store$|nohup\.out$)'
for f in "${files[@]}"; do
  if printf '%s' "$f" | grep -Eiq "$BAD_NAME"; then
    echo "ЗАПРЕЩЁННЫЙ ФАЙЛ: $f"
    fail=1
  fi
  if [ -f "$f" ] && [ "$(wc -c <"$f")" -gt 5000000 ]; then
    echo "СЛИШКОМ БОЛЬШОЙ (>5 МБ): $f"
    fail=1
  fi
done

# 2. Содержимое, похожее на секреты (только имена файлов)
SECRET_RE='(AIza[0-9A-Za-z_-]{30,}|ghp_[0-9A-Za-z]{30,}|github_pat_[0-9A-Za-z_]{30,}|gho_[0-9A-Za-z]{30,}|xox[baprs]-[0-9A-Za-z-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|client_secret["'"'"' ]*[:=]["'"'"' ]*[A-Za-z0-9_-]{10,}|SAPISID=[A-Za-z0-9_/-]{10,}|__Secure-[0-9A-Z]*PSID=[A-Za-z0-9_/-]{10,}|"google_cookie": *"[^"]{10,}|(password|passwd|storePassword|keyPassword)["'"'"' ]*[:=]["'"'"' ]*[^"'"'"' ]{4,})'
hits=$(git grep --cached -lIE -e "$SECRET_RE" -- "${files[@]}" "$self" 2>/dev/null || true)
if [ -n "$hits" ]; then
  echo "ПОХОЖЕ НА СЕКРЕТЫ (проверь вручную, значения не показываю):"
  printf '  %s\n' $hits
  fail=1
fi

# 3. Личные данные (предупреждение)
PERSONAL_RE='(/home/[a-z0-9_.-]+/|/Users/[A-Za-z0-9_.-]+/|C:\\Users\\[^\\]+\\|[A-Za-z0-9._%+-]+@(gmail|proton|protonmail|yandex|mail|outlook|icloud)\.[a-z]+|\b([0-9]{1,3}\.){3}[0-9]{1,3}\b)'
phits=$(git grep --cached -lIE -e "$PERSONAL_RE" -- "${files[@]}" "$self" 2>/dev/null || true)
if [ -n "$phits" ]; then
  echo "ВНИМАНИЕ: возможно личные данные (путь домашней папки, email, IP):"
  printf '  %s\n' $phits
  warn=1
fi

if [ "$fail" -ne 0 ]; then
  echo
  echo "ПУБЛИКАЦИЯ ЗАПРЕЩЕНА. Убери файлы из индекса (git restore --staged <файл>) или спроси пользователя."
  echo "Если секрет уже попал в коммит или был запушен: остановись, сообщи пользователю, секрет считается скомпрометированным."
  exit 1
fi
if [ "$warn" -ne 0 ]; then
  echo
  echo "Предупреждения не блокируют, но проверь каждый файл из списка и подтверди, что это не личные данные."
fi
echo "OK: запрещённого не найдено."
exit 0
