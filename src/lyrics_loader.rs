use crate::app::{AppEvent, MetrolistApp};
use crate::models::Track;
use std::sync::Arc;

impl MetrolistApp {
    pub(crate) fn start_lyrics_search_for_track(&mut self, track: &Track, bypass_cache: bool) {
        self.is_local_lyrics = false;
        self.lyrics_req_id = self.lyrics_req_id.wrapping_add(1);
        let req_id = self.lyrics_req_id;
        let track_id = track.id.clone();
        let title = track.title.clone();
        let artist = track.artist.clone();
        let mode = self.settings.lyrics_source_mode;
        let client = Arc::clone(&self.lyrics_client);
        let tx = self.event_tx.clone();
        let dur_secs = crate::lyrics::LrcParser::parse_timestamp(&track.duration_text)
            .map(|ms| ms / 1000)
            .filter(|secs| *secs > 0);

        tokio::spawn(async move {
            // 1. Проверяем локальный кэш соответствующего режима (если поиск не принудительно без кэша)
            if !bypass_cache {
                if let Some(cached) = client.get_cached(mode, &title, &artist, dur_secs).await {
                    let has_word_sync = cached.is_word_synced();
                    let _ = tx.send(AppEvent::LyricsLoaded {
                        track_id: track_id.clone(),
                        req_id,
                        lyrics: Some(cached),
                        is_upgraded: false,
                    });
                    if has_word_sync || mode != 0 {
                        return;
                    }
                }
            }

            // 2. Если выбран конкретный источник вручную
            if mode != 0 {
                let res = client
                    .fetch_direct_source(mode, &title, &artist, None, dur_secs)
                    .await;
                if let Some(ref l) = res {
                    client
                        .save_to_cache(mode, &title, &artist, dur_secs, l)
                        .await;
                }
                let _ = tx.send(AppEvent::LyricsLoaded {
                    track_id,
                    req_id,
                    lyrics: res,
                    is_upgraded: false,
                });
                return;
            }

            // 3. Автоматический режим: сначала LRCLIB с переданной длительностью и метаданными
            let primary = client
                .fetch_with_meta(&title, &artist, None, dur_secs)
                .await;
            let mut had_primary = false;
            if let Some(ref l) = primary {
                had_primary = true;
                let _ = tx.send(AppEvent::LyricsLoaded {
                    track_id: track_id.clone(),
                    req_id,
                    lyrics: Some(l.clone()),
                    is_upgraded: false,
                });
                if l.is_word_synced() {
                    client.save_to_cache(0, &title, &artist, dur_secs, l).await;
                    return;
                }
            }

            // 4. В фоне: ищем более подробную пословную/слоговую синхронизацию (TTML, QQ, LrcMux)
            if let Some(richer) = client
                .fetch_richer_sync(&title, &artist, None, dur_secs)
                .await
            {
                client
                    .save_to_cache(0, &title, &artist, dur_secs, &richer)
                    .await;
                let _ = tx.send(AppEvent::LyricsLoaded {
                    track_id,
                    req_id,
                    lyrics: Some(richer),
                    is_upgraded: true,
                });
            } else if had_primary {
                if let Some(ref l) = primary {
                    client.save_to_cache(0, &title, &artist, dur_secs, l).await;
                }
            } else {
                // Если LRCLIB и пословные не нашли, пробуем KuGou как резерв построчного текста
                if let Some(fallback) = client
                    .fetch_direct_source(4, &title, &artist, None, dur_secs)
                    .await
                {
                    client
                        .save_to_cache(0, &title, &artist, dur_secs, &fallback)
                        .await;
                    let _ = tx.send(AppEvent::LyricsLoaded {
                        track_id,
                        req_id,
                        lyrics: Some(fallback),
                        is_upgraded: false,
                    });
                } else {
                    let _ = tx.send(AppEvent::LyricsLoaded {
                        track_id,
                        req_id,
                        lyrics: None,
                        is_upgraded: false,
                    });
                }
            }
        });
        self.lyrics = None;
    }

    pub(crate) fn trigger_lyrics_search(&mut self, bypass_cache: bool) {
        self.is_local_lyrics = false;
        if let Some(track) = self.current_track().cloned() {
            self.start_lyrics_search_for_track(&track, bypass_cache);
        }
    }

    pub(crate) fn pick_and_load_local_lyrics_file(&self) {
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let output = std::process::Command::new("zenity")
                .args([
                    "--file-selection",
                    "--title=Выберите файл текста песни (.lrc, .ttml)",
                    "--file-filter=Тексты песен (*.lrc *.ttml) | *.lrc *.ttml",
                ])
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        let _ = tx.send(AppEvent::LocalLyricsFileSelected(
                            std::path::PathBuf::from(path_str),
                        ));
                    }
                }
            }
        });
    }
}
