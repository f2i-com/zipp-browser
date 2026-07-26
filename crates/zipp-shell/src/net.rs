//! Networking (HTTP, filesystem, Data URIs) for the Zipp browser engine.
//!
//! Provides [`Provider`], an implementation of the
//! [`zipp_dom::traits_src::net::NetProvider`] trait that fetches resources
//! over HTTP (via reqwest), from the local filesystem, or from `data:` URIs.
//! An optional HTTP response cache is available behind the `cache` feature.

// use zipp_dom::traits_src::net::{Body, Bytes, NetHandler, NetProvider, NetWaker, Request};
use zipp_dom::traits_src::net::{AbortSignal, Body, Bytes, NetHandler, NetProvider, NetWaker, Request};
use data_url::DataUrl;
use std::{marker::PhantomData, pin::Pin, sync::Arc, task::Poll};

#[cfg(feature = "cache")]
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};

/// Default User-Agent string. Mimics a modern Chrome browser for maximum site compatibility.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0";

#[cfg(feature = "cache")]
type Client = reqwest_middleware::ClientWithMiddleware;
#[cfg(not(feature = "cache"))]
type Client = reqwest::Client;

#[cfg(feature = "cache")]
type RequestBuilder = reqwest_middleware::RequestBuilder;
#[cfg(not(feature = "cache"))]
type RequestBuilder = reqwest::RequestBuilder;

#[cfg(feature = "cache")]
fn get_cache_path() -> std::path::PathBuf {
    use directories::ProjectDirs;
    let path = ProjectDirs::from("com", "Zipp", "Zipp")
        .expect("Failed to find cache directory")
        .cache_dir()
        .to_owned();
    #[cfg(feature = "tracing")]
    tracing::info!(path = ?path.display(), "Using cache dir");
    path
}

#[cfg(target_arch = "wasm32")]
fn spawn(fut: impl Future + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        fut.await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn<F>(fut: F)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(fut);
}

pub struct Provider {
    client: Client,
    waker: Arc<dyn NetWaker>,
    user_agent: String,
}
impl Provider {
    /// Create a new network provider with the default User-Agent.
    pub fn new(waker: Option<Arc<dyn NetWaker>>) -> Self {
        Self::with_user_agent(waker, DEFAULT_USER_AGENT.to_owned())
    }

    /// Create a new network provider with a custom User-Agent string.
    pub fn with_user_agent(waker: Option<Arc<dyn NetWaker>>, user_agent: String) -> Self {
        let builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .danger_accept_invalid_certs(true); // Fix TLS on RDP/corporate environments
        #[cfg(feature = "cookies")]
        let builder = builder.cookie_store(true);
        let client = builder.build().unwrap();

        #[cfg(feature = "cache")]
        let client = reqwest_middleware::ClientBuilder::new(client)
            .with(Cache(HttpCache {
                mode: CacheMode::Default,
                manager: CACacheManager::new(get_cache_path(), true),
                options: HttpCacheOptions::default(),
            }))
            .build();

        let waker = waker.unwrap_or(Arc::new(NoOpNetWaker));
        Self { client, waker, user_agent }
    }

    /// Returns the current User-Agent string.
    #[inline]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
    #[inline]
    pub fn shared(waker: Option<Arc<dyn NetWaker>>) -> Arc<dyn NetProvider> {
        Arc::new(Self::new(waker))
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        Arc::strong_count(&self.waker) == 1
    }
    #[inline]
    pub fn count(&self) -> usize {
        Arc::strong_count(&self.waker) - 1
    }
}
impl Provider {
    async fn fetch_inner(
        client: Client,
        request: Request,
        user_agent: String,
    ) -> Result<(String, Bytes), ProviderError> {
        Ok(match request.url.scheme() {
            "data" => {
                let url_str = request.url.as_str();
                let data_url = DataUrl::process(url_str)?;
                let decoded = data_url.decode_to_vec()?;
                // Truncate data URLs in the returned identifier to avoid huge
                // string copies (callers only use this for logging/identity).
                let id = if url_str.len() > 64 {
                    url_str[..64].to_string()
                } else {
                    url_str.to_string()
                };
                (id, Bytes::from(decoded.0))
            }
            "file" => {
                let file_content = std::fs::read(request.url.path())?;
                (request.url.to_string(), Bytes::from(file_content))
            }
            _ => {
                let response = client
                    .request(request.method, request.url)
                    .headers(request.headers)
                    .header("Content-Type", request.content_type.as_str())
                    .header("User-Agent", user_agent.as_str())
                    .apply_body(request.body, request.content_type.as_str())
                    .await
                    .send()
                    .await?;

                (response.url().to_string(), response.bytes().await?)
            }
        })
    }

    #[allow(clippy::type_complexity)]
    pub fn fetch_with_callback(
        &self,
        request: Request,
        callback: Box<dyn FnOnce(Result<(String, Bytes), ProviderError>) + Send + Sync + 'static>,
    ) {
        #[cfg(feature = "tracing")]
        let url = request.url.to_string();

        let client = self.client.clone();
        let ua = self.user_agent.clone();
        spawn(async move {
            let result = Self::fetch_inner(client, request, ua).await;

            #[cfg(feature = "tracing")]
            if let Err(e) = &result {
                #[cfg(feature = "tracing")]
                tracing::error!(url = url.as_str(), error = ?e, "Fetching");
            } else {
                #[cfg(feature = "tracing")]
                tracing::info!(url = url.as_str(), "Success fetching");
            }

            callback(result);
        });
    }

    pub async fn fetch_async(&self, request: Request) -> Result<(String, Bytes), ProviderError> {
        #[cfg(feature = "tracing")]
        let url = request.url.to_string();

        let client = self.client.clone();
        let result = Self::fetch_inner(client, request, self.user_agent.clone()).await;

        #[cfg(feature = "tracing")]
        if let Err(e) = &result {
            #[cfg(feature = "tracing")]
            tracing::error!(url = url.as_str(), error = ?e, "Fetching");
        } else {
            #[cfg(feature = "tracing")]
            tracing::info!(url = url.as_str(), "Success fetching");
        }

        result
    }
}

impl NetProvider for Provider {
    fn fetch(&self, doc_id: usize, mut request: Request, handler: Box<dyn NetHandler>) {
        let client = self.client.clone();
        let ua = self.user_agent.clone();

        #[cfg(feature = "tracing")]
        tracing::info!(url = request.url.as_str(), "Fetching");

        let waker = self.waker.clone();
        spawn(async move {
            #[cfg(feature = "tracing")]
            let _url = request.url.to_string();

            let signal = request.signal.take();
            let result = if let Some(signal) = signal {
                let ua2 = ua.clone();
                AbortFetch::new(
                    signal,
                    Box::pin(async move { Self::fetch_inner(client, request, ua2).await }),
                )
                .await
            } else {
                Self::fetch_inner(client, request, ua).await
            };

            // Call the waker to notify of completed network request
            waker.wake(doc_id);

            match result {
                Ok((response_url, bytes)) => {
                    eprintln!("[NET] Fetched {} ({} bytes)", response_url, bytes.len());
                    handler.bytes(response_url, bytes);
                }
                Err(e) => {
                    eprintln!("[NET] Fetch error: {e:?}");
                }
            };
        });
    }
}

/// A future that is cancellable using an AbortSignal
struct AbortFetch<F, T> {
    signal: AbortSignal,
    future: F,
    _rt: PhantomData<T>,
}

impl<F, T> AbortFetch<F, T> {
    fn new(signal: AbortSignal, future: F) -> Self {
        Self {
            signal,
            future,
            _rt: PhantomData,
        }
    }
}

impl<F, T> Future for AbortFetch<F, T>
where
    F: Future + Unpin + 'static,
    F::Output: Into<Result<T, ProviderError>> + 'static,
    T: Unpin,
{
    type Output = Result<T, ProviderError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.signal.aborted() {
            return Poll::Ready(Err(ProviderError::Abort));
        }

        match Pin::new(&mut self.future).poll(cx) {
            Poll::Ready(output) => Poll::Ready(output.into()),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
pub enum ProviderError {
    Abort,
    Io(std::io::Error),
    DataUrl(data_url::DataUrlError),
    DataUrlBase64(data_url::forgiving_base64::InvalidBase64),
    ReqwestError(reqwest::Error),
    #[cfg(feature = "cache")]
    ReqwestMiddlewareError(reqwest_middleware::Error),
}

impl From<std::io::Error> for ProviderError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<data_url::DataUrlError> for ProviderError {
    fn from(value: data_url::DataUrlError) -> Self {
        Self::DataUrl(value)
    }
}

impl From<data_url::forgiving_base64::InvalidBase64> for ProviderError {
    fn from(value: data_url::forgiving_base64::InvalidBase64) -> Self {
        Self::DataUrlBase64(value)
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(value: reqwest::Error) -> Self {
        Self::ReqwestError(value)
    }
}

#[cfg(feature = "cache")]
impl From<reqwest_middleware::Error> for ProviderError {
    fn from(value: reqwest_middleware::Error) -> Self {
        Self::ReqwestMiddlewareError(value)
    }
}

trait ReqwestExt {
    async fn apply_body(self, body: Body, content_type: &str) -> Self;
}
impl ReqwestExt for RequestBuilder {
    async fn apply_body(self, body: Body, content_type: &str) -> Self {
        match body {
            Body::Bytes(bytes) => self.body(bytes),
            Body::Form(form_data) => match content_type {
                "application/x-www-form-urlencoded" => self.form(&form_data),
                #[cfg(feature = "multipart")]
                "multipart/form-data" => {
                    use zipp_dom::traits_src::net::Entry;
                    use zipp_dom::traits_src::net::EntryValue;
                    let mut form_data = form_data;
                    let mut form = reqwest::multipart::Form::new();
                    for Entry { name, value } in form_data.0.drain(..) {
                        form = match value {
                            EntryValue::String(value) => form.text(name, value),
                            EntryValue::File(path_buf) => form
                                .file(name, path_buf)
                                .await
                                .expect("Couldn't read form file from disk"),
                            EntryValue::EmptyFile => form.part(
                                name,
                                reqwest::multipart::Part::bytes(&[])
                                    .mime_str("application/octet-stream")
                                    .unwrap(),
                            ),
                        };
                    }
                    self.multipart(form)
                }
                _ => self,
            },
            Body::Empty => self,
        }
    }
}

/// A [`NetWaker`] that does nothing, used as the default when no waker is supplied.
struct NoOpNetWaker;
impl NetWaker for NoOpNetWaker {
    #[inline]
    fn wake(&self, _client_id: usize) {}
}
