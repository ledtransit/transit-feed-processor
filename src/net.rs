use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::{fs, io};

use anyhow::{Context, Result, anyhow, bail};
use log::warn;
use reqwest::header::ETAG;
use reqwest::{Identity, RequestBuilder, Response, StatusCode, Url};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::time::{Duration, Instant, sleep};

use crate::feed::RemoteSource;

/// The placeholder shipped in `.env.example`; rejected so operators can't run
/// with an anonymous, unactionable identity.
const PLACEHOLDER_USER_AGENT: &str = "transit-feed-processor (+https://example.com)";

/// `User-Agent` for outbound feed requests, read from the **required**
/// `USER_AGENT` env var (see `.env.example`). Feed publishers use it to see who
/// is calling and how to reach them, so a missing, empty, or still-default value
/// is a hard error rather than a silent fallback.
pub fn user_agent() -> Result<String> {
    let ua = std::env::var("USER_AGENT")
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .context(
            "USER_AGENT must be set to a User-Agent that identifies who is making \
             feed requests and how to reach them (see .env.example)",
        )?;
    if ua == PLACEHOLDER_USER_AGENT {
        bail!(
            "USER_AGENT is still the .env.example placeholder; set a real value that \
             identifies the operator (e.g. a contact URL or email)"
        );
    }
    Ok(ua)
}

pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(user_agent()?)
        .build()
        .context("build http client")
}

/// Build a GET request for `src` with its credentials attached, to be sent with
/// [`fetch_retrying`] which applies `src.timeout`. Returns
/// `Ok(None)` when the source needs credentials that aren't available (env var
/// unset or cert file missing), so the caller can skip it.
pub fn get(src: &RemoteSource) -> Result<Option<RequestBuilder>> {
    if !src.needs_auth() {
        return Ok(Some(client()?.get(src.url)));
    }
    Ok(authed(src)?.map(|a| a.client.get(a.url)))
}

/// Client and url with a source's credentials applied.
#[derive(Clone)]
struct Authed {
    client: reqwest::Client,
    url: Url,
}

/// Credentials per source url, loaded on first use and reused so the cert isn't
/// re-imported on every poll and a missing credential is only warned about once.
static AUTHED: LazyLock<Mutex<HashMap<&'static str, Option<Authed>>>> =
    LazyLock::new(Default::default);

fn authed(src: &RemoteSource) -> Result<Option<Authed>> {
    let mut cache = AUTHED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = cache.get(src.url) {
        return Ok(cached.clone());
    }
    let authed = load_authed(src)?;
    cache.insert(src.url, authed.clone());
    Ok(authed)
}

fn load_authed(src: &RemoteSource) -> Result<Option<Authed>> {
    let mut url = Url::parse(src.url).with_context(|| format!("parse url {}", src.url))?;
    if let Some(q) = &src.query_auth {
        let Some(value) = secret_env(q.env) else {
            warn!("{} is not set, skipping {}", q.env, src.url);
            return Ok(None);
        };
        url.query_pairs_mut().append_pair(q.param, &value);
    }

    let mut builder = reqwest::Client::builder().user_agent(user_agent()?);
    if let Some(cert) = &src.client_cert {
        let Some(password) = secret_env(cert.password_env) else {
            warn!("{} is not set, skipping {}", cert.password_env, src.url);
            return Ok(None);
        };
        let der = match fs::read(cert.path) {
            Ok(der) => der,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                warn!(
                    "client certificate {} not found, skipping {}",
                    cert.path, src.url
                );
                return Ok(None);
            }
            Err(e) => return Err(e).with_context(|| format!("read {}", cert.path)),
        };
        let identity = Identity::from_pkcs12_der(&der, &password)
            .with_context(|| format!("load client certificate {}", cert.path))?;
        builder = builder.identity(identity);
    }

    let client = builder.build().context("build http client")?;
    Ok(Some(Authed { client, url }))
}

fn secret_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A successfully fetched response: status, `ETag` (if any) and the full body.
pub struct Fetched {
    pub status: StatusCode,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

/// Send `req` and read its full body, retrying transient failures (5xx, 429,
/// timeouts, connection errors) up to `src.max_retries` times with exponential
/// backoff. Retries cover the *whole* attempt — connecting, sending, and
/// streaming the body — since for a large feed most of the wall-clock time
/// (and risk of a transient hiccup) is spent reading the body, not sending
/// the request. `src.timeout` is a hard deadline for all attempts together:
/// each attempt only gets the time left, and no retry starts once it's used
/// up. A 2xx or 304 response is returned as-is; any other status is an error.
pub async fn fetch_retrying(req: RequestBuilder, src: &RemoteSource) -> Result<Fetched> {
    retrying(req, src, try_fetch).await
}

/// A response whose body was streamed to a file: status, `ETag` (if any),
/// and the body's size and SHA-256 (hex). A 304 wrote nothing.
pub struct Downloaded {
    pub status: StatusCode,
    pub etag: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}

/// Like [`fetch_retrying`], but streams the body to `path` (truncated on every
/// attempt) as it arrives instead of holding it in memory, hashing it on the
/// way: for static feeds of hundreds of megabytes.
pub async fn download_retrying(
    req: RequestBuilder,
    src: &RemoteSource,
    path: &Path,
) -> Result<Downloaded> {
    retrying(req, src, |call| try_download(call, path)).await
}

/// Run `attempt` on `req`, retrying transient failures as described on
/// [`fetch_retrying`].
async fn retrying<T, F, Fut>(
    req: RequestBuilder,
    src: &RemoteSource,
    mut attempt_with: F,
) -> Result<T>
where
    F: FnMut(RequestBuilder) -> Fut,
    Fut: Future<Output = Result<T, Failure>>,
{
    let retries = src.max_retries;
    let deadline = Instant::now() + src.timeout;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let call = req
            .try_clone()
            .context("request cannot be retried")?
            .timeout(remaining);
        match attempt_with(call).await {
            Ok(fetched) => return Ok(fetched),
            Err(Failure::Retriable(msg)) if attempt <= retries => {
                let wait = backoff(attempt);
                if Instant::now() + wait >= deadline {
                    bail!("{msg}, no time left to retry within {:?}", src.timeout);
                }
                warn!("{msg}, retry {attempt}/{retries}");
                sleep(wait).await;
            }
            Err(Failure::Retriable(msg)) => bail!("{msg}, giving up after {retries} retries"),
            Err(Failure::Fatal(e)) => return Err(e),
        }
    }
}

enum Failure {
    Retriable(String),
    Fatal(anyhow::Error),
}

/// Send `call`, failing on any status but 2xx and 304.
async fn send(call: RequestBuilder) -> Result<(Response, StatusCode, Option<String>), Failure> {
    let resp = call
        .send()
        .await
        .map_err(|e| classify(e, "http request failed"))?;
    let status = resp.status();
    if !status.is_success() && status != StatusCode::NOT_MODIFIED {
        return Err(if is_retriable_status(status) {
            Failure::Retriable(format!("http {status}"))
        } else {
            Failure::Fatal(anyhow!("unexpected http status {status}"))
        });
    }
    let etag = resp
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    Ok((resp, status, etag))
}

async fn try_fetch(call: RequestBuilder) -> Result<Fetched, Failure> {
    let (resp, status, etag) = send(call).await?;
    let body = resp
        .bytes()
        .await
        .map_err(|e| classify(e, "read response body"))?
        .to_vec();
    Ok(Fetched { status, etag, body })
}

async fn try_download(call: RequestBuilder, path: &Path) -> Result<Downloaded, Failure> {
    let (mut resp, status, etag) = send(call).await?;
    let mut downloaded = Downloaded {
        status,
        etag,
        bytes: 0,
        sha256: String::new(),
    };
    if status == StatusCode::NOT_MODIFIED {
        return Ok(downloaded);
    }

    let fatal = |e: io::Error| {
        Failure::Fatal(anyhow::Error::new(e).context(format!("write {}", path.display())))
    };
    let mut file = tokio::fs::File::create(path).await.map_err(fatal)?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| classify(e, "read response body"))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(fatal)?;
        downloaded.bytes += chunk.len() as u64;
    }
    file.flush().await.map_err(fatal)?;
    downloaded.sha256 = hex(&hasher.finalize());
    Ok(downloaded)
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn classify(e: reqwest::Error, context: &str) -> Failure {
    let retriable = is_retriable_err(&e);
    // Drop the url: it may carry a secret query parameter.
    let err = anyhow::Error::new(e.without_url()).context(context.to_owned());
    if retriable {
        Failure::Retriable(format!("{err:#}"))
    } else {
        Failure::Fatal(err)
    }
}

fn is_retriable_status(s: StatusCode) -> bool {
    s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS
}

fn is_retriable_err(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request() || e.is_body()
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * 2u64.pow((attempt - 1).min(5)))
}
