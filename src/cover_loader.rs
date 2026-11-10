use eframe::egui::{
    self,
    load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

enum Cover {
    Loading,
    Ready {
        bytes: Arc<[u8]>,
        mime: Option<String>,
    },
    Failed,
}

pub(crate) struct CoverLoader {
    client: reqwest::Client,
    covers: Arc<egui::mutex::Mutex<HashMap<String, Cover>>>,
}

impl CoverLoader {
    pub(crate) fn install(ctx: &egui::Context) {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        // egui tries the most recently registered bytes loader first.
        ctx.add_bytes_loader(Arc::new(Self {
            client,
            covers: Default::default(),
        }));
    }

    async fn fetch(client: &reqwest::Client, url: &str) -> Result<Cover, reqwest::Error> {
        const MAX_BYTES: usize = 12 * 1024 * 1024;
        let mut response = client
            .get(url)
            .timeout(Duration::from_secs(8))
            .header(
                reqwest::header::ACCEPT,
                "image/jpeg,image/png,image/gif,image/svg+xml",
            )
            .send()
            .await?
            .error_for_status()?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Ok(Cover::Failed);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Cover::Ready {
            bytes: bytes.into(),
            mime,
        })
    }
}

impl BytesLoader for CoverLoader {
    fn id(&self) -> &str {
        "cibitube-cover-http"
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("https://") && !uri.starts_with("http://") {
            return Err(LoadError::NotSupported);
        }
        let mut covers = self.covers.lock();
        match covers.get(uri) {
            Some(Cover::Ready { bytes, mime }) => {
                return Ok(BytesPoll::Ready {
                    size: None,
                    bytes: Bytes::Shared(Arc::clone(bytes)),
                    mime: mime.clone(),
                })
            }
            Some(Cover::Failed) => return Err(LoadError::Loading("Обложка недоступна".into())),
            Some(Cover::Loading) => return Ok(BytesPoll::Pending { size: None }),
            None => {
                covers.insert(uri.to_owned(), Cover::Loading);
            }
        }
        drop(covers);

        let client = self.client.clone();
        let covers = Arc::clone(&self.covers);
        let uri = uri.to_owned();
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let result = match Self::fetch(&client, &uri).await {
                Err(error)
                    if error.is_connect()
                        || error.is_timeout()
                        || error
                            .status()
                            .is_some_and(|status| status.is_server_error()) =>
                {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Self::fetch(&client, &uri).await
                }
                other => other,
            };
            let mut covers = covers.lock();
            if let Some(cover @ Cover::Loading) = covers.get_mut(&uri) {
                *cover = result.unwrap_or(Cover::Failed);
            }
            drop(covers);
            ctx.request_repaint();
        });
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.covers.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.covers.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.covers
            .lock()
            .values()
            .map(|cover| match cover {
                Cover::Ready { bytes, .. } => bytes.len(),
                _ => 0,
            })
            .sum()
    }
}
