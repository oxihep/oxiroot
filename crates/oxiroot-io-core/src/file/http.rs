//! An HTTP(S) byte-range [`ByteSource`] (the `http` feature).
//!
//! Reads a ROOT file served over HTTP without downloading it whole: each
//! [`read_at`](ByteSource::read_at) issues a `Range: bytes=…` request and
//! returns just that slice, exactly as ROOT and uproot read remote files. The
//! server must honor range requests (`Accept-Ranges: bytes`, `206 Partial
//! Content`). Small (metadata) reads are cached so re-parsing the streamer info
//! or key list does not re-fetch.

use crate::cast::SaturatingFrom;
use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;

use bytes::Bytes;

use super::source::ByteSource;
use crate::error::{Error, Result};

/// Reads at or below this size are cached (file header, directory, key list,
/// streamer info). Large page/basket reads are fetched once by the readers and
/// not cached, so the cache cannot grow without bound.
const CACHE_MAX_ENTRY: usize = 64 * 1024;

/// A ROOT file accessed over HTTP(S) with byte-range requests.
pub struct HttpSource {
    agent: ureq::Agent,
    url: String,
    len: u64,
    cache: Mutex<HashMap<(u64, usize), Bytes>>,
}

impl std::fmt::Debug for HttpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSource")
            .field("url", &self.url)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl HttpSource {
    /// Open `url`, discovering the file length and confirming range support via a
    /// one-byte ranged request.
    pub fn open(url: &str) -> Result<Self> {
        let agent = ureq::Agent::new_with_defaults();
        // A one-byte ranged GET both proves the server honors `Range` and yields
        // the total length from the `Content-Range` header.
        let res = agent
            .get(url)
            .header("Range", "bytes=0-0")
            .call()
            .map_err(|e| http_err(&format!("opening {url}"), &e))?;
        let status = res.status().as_u16();
        if status != 206 {
            // A 200 means the server ignored the Range header.
            let kind = match status {
                200 => std::io::ErrorKind::Unsupported,
                _ => status_kind(status),
            };
            return Err(Error::Io(std::io::Error::new(
                kind,
                format!(
                    "remote {url}: server must support HTTP Range requests \
                     (expected 206 Partial Content, got {status})"
                ),
            )));
        }
        let total = content_range_total(&res, url)?;
        // Drain the (one-byte) body so the connection returns to the pool.
        let _ = res.into_body().into_reader().read_to_end(&mut Vec::new());
        Ok(Self {
            agent,
            url: url.to_string(),
            len: total,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// [`fetch`](Self::fetch), after checking the range lies in the file.
    fn checked_fetch(&self, offset: u64, len: usize) -> Result<Bytes> {
        if len == 0 {
            return Ok(Bytes::new());
        }
        offset
            .checked_add(len as u64)
            .filter(|&e| e <= self.len)
            .ok_or_else(|| Error::UnexpectedEof {
                needed: len,
                available: usize::saturating_from(self.len.saturating_sub(offset)),
            })?;
        self.fetch(offset, len)
    }

    /// Issue a range request for `[offset, offset + len)` and return its bytes.
    fn fetch(&self, offset: u64, len: usize) -> Result<Bytes> {
        let end = offset + len as u64 - 1;
        let range = format!("bytes={offset}-{end}");
        let res = self
            .agent
            .get(&self.url)
            .header("Range", &range)
            .call()
            .map_err(|e| http_err(&format!("range {range} of {}", self.url), &e))?;
        let status = res.status().as_u16();
        if status != 206 && status != 200 {
            return Err(Error::Io(std::io::Error::new(
                status_kind(status),
                format!("remote {}: range {range} returned HTTP {status}", self.url),
            )));
        }
        let mut buf = Vec::with_capacity(len);
        res.into_body()
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("reading range {range}: {e}"),
                ))
            })?;
        if buf.len() != len {
            return Err(Error::UnexpectedEof {
                needed: len,
                available: buf.len(),
            });
        }
        Ok(Bytes::from(buf))
    }
}

/// The most range requests [`HttpSource::read_many`] keeps in flight.
const MAX_CONCURRENT: usize = 8;

impl ByteSource for HttpSource {
    fn len(&self) -> u64 {
        self.len
    }

    /// Fetch the ranges with up to [`MAX_CONCURRENT`] requests in flight, so a
    /// batch costs about one round trip per eight ranges rather than one each.
    fn read_many(&self, ranges: &[(u64, usize)]) -> Result<Vec<Bytes>> {
        if ranges.len() <= 1 {
            return ranges.iter().map(|&(o, l)| self.read_at(o, l)).collect();
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results: Vec<Mutex<Option<Result<Bytes>>>> =
            ranges.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..MAX_CONCURRENT.min(ranges.len()) {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(offset, len)) = ranges.get(i) else {
                            break;
                        };
                        // Fetched directly, not through `read_at`: batched
                        // basket and page bytes are read once, so they stay out
                        // of the metadata cache.
                        let r = self.checked_fetch(offset, len);
                        let failed = r.is_err();
                        *results[i]
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);
                        if failed {
                            // Stop taking work; the error is returned below.
                            next.store(ranges.len(), std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            }
        });
        results
            .into_iter()
            .map(|r| {
                r.into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .unwrap_or_else(|| {
                        Err(Error::Io(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            format!("remote {}: a batched range was not fetched", self.url),
                        )))
                    })
            })
            .collect()
    }

    fn batches_reads(&self) -> bool {
        true
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        if len == 0 {
            return Ok(Bytes::new());
        }
        offset
            .checked_add(len as u64)
            .filter(|&e| e <= self.len)
            .ok_or_else(|| Error::UnexpectedEof {
                needed: len,
                available: usize::saturating_from(self.len.saturating_sub(offset)),
            })?;

        let cacheable = len <= CACHE_MAX_ENTRY;
        if cacheable
            && let Some(hit) = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&(offset, len))
                .cloned()
        {
            return Ok(hit);
        }
        let bytes = self.fetch(offset, len)?;
        if cacheable {
            self.cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert((offset, len), bytes.clone());
        }
        Ok(bytes)
    }
}

/// Parse the total file length out of a `206` response's `Content-Range`
/// (`bytes 0-0/<total>`).
fn content_range_total(res: &ureq::http::Response<ureq::Body>, url: &str) -> Result<u64> {
    let cr = res
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("remote {url}: 206 without Content-Range"),
            ))
        })?;
    cr.rsplit('/')
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("remote {url}: unparseable Content-Range {cr:?}"),
            ))
        })
}

/// The I/O error kind an HTTP error status stands for.
fn status_kind(status: u16) -> std::io::ErrorKind {
    match status {
        404 | 410 => std::io::ErrorKind::NotFound,
        401 | 403 => std::io::ErrorKind::PermissionDenied,
        _ => std::io::ErrorKind::Other,
    }
}

/// Map a ureq transport error to an [`Error`] with context.
fn http_err(ctx: &str, e: &ureq::Error) -> Error {
    Error::Io(std::io::Error::other(format!("{ctx}: {e}")))
}
