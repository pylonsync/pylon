//! Compression of dynamic response bodies: API JSON and sync pulls.
//!
//! Static client files are compressed once at build time (see
//! `packages/functions/src/ssr-precompress.ts`). Dynamic bodies are
//! compressed per response, with brotli at a speed-oriented quality when
//! the client accepts `br`, else gzip. A sync pull page of 3.3 MB of JSON
//! is about 0.6 MB on the wire.

use std::io::Write;

/// Bodies smaller than this are sent as they are: the saving does not
/// cover the encoding headers.
pub const MIN_COMPRESS_BYTES: usize = 1024;

/// A content coding this module produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Brotli,
    Gzip,
}

impl Encoding {
    /// The `Content-Encoding` value.
    pub fn header_value(self) -> &'static str {
        match self {
            Encoding::Brotli => "br",
            Encoding::Gzip => "gzip",
        }
    }
}

/// Whether an `Accept-Encoding` value accepts `coding` (RFC 9110
/// §12.5.3): listed, or covered by `*`, with a non-zero q-value. An
/// explicit entry for the coding wins over `*`.
pub fn accepts_encoding(header: &str, coding: &str) -> bool {
    let mut wildcard = None;
    for item in header.split(',') {
        let mut parts = item.split(';');
        let name = parts.next().unwrap_or("").trim();
        let q = parts
            .filter_map(|p| p.trim().strip_prefix("q="))
            .find_map(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        if name.eq_ignore_ascii_case(coding) {
            return q > 0.0;
        }
        if name == "*" {
            wildcard = Some(q > 0.0);
        }
    }
    wildcard.unwrap_or(false)
}

/// The encoding to use for a client's `Accept-Encoding`: brotli, then
/// gzip, else none.
pub fn negotiate(accept_encoding: Option<&str>) -> Option<Encoding> {
    let accept = accept_encoding?;
    if accepts_encoding(accept, "br") {
        Some(Encoding::Brotli)
    } else if accepts_encoding(accept, "gzip") {
        Some(Encoding::Gzip)
    } else {
        None
    }
}

/// Whether a response of this content type is worth compressing: JSON,
/// text, JavaScript, XML.
pub fn compressible_content_type(content_type: &str) -> bool {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    ct.starts_with("text/")
        || ct == "application/json"
        || ct.ends_with("+json")
        || ct == "application/javascript"
        || ct == "application/xml"
        || ct.ends_with("+xml")
}

/// `body` compressed with `encoding`, or `None` when it is smaller than
/// `MIN_COMPRESS_BYTES`, the encoder failed, or the result is not smaller.
pub fn compress(body: &[u8], encoding: Encoding) -> Option<Vec<u8>> {
    if body.len() < MIN_COMPRESS_BYTES {
        return None;
    }
    let out = match encoding {
        Encoding::Brotli => {
            // Quality 5 with a 4 MB window: most of quality 11's ratio on
            // JSON at a small fraction of its time, which matters per
            // response.
            let mut out = Vec::with_capacity(body.len() / 4);
            {
                let mut writer = brotli::CompressorWriter::new(&mut out, 64 * 1024, 5, 22);
                writer.write_all(body).ok()?;
                writer.flush().ok()?;
            }
            out
        }
        Encoding::Gzip => {
            let mut encoder = flate2::write::GzEncoder::new(
                Vec::with_capacity(body.len() / 4),
                flate2::Compression::new(6),
            );
            encoder.write_all(body).ok()?;
            encoder.finish().ok()?
        }
    };
    (out.len() < body.len()).then_some(out)
}

/// A compressed streaming body: reads chunks from `rx` (each one a piece of
/// a streamed SSR render), compresses them, and flushes the encoder after
/// every chunk. Each flush ends a complete block the browser can decode at
/// once, so the page still paints Suspense boundary by boundary; the cost
/// is a few bytes per flush. An empty chunk, or the sender dropping, ends
/// the stream.
pub struct StreamingEncoder {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    encoder: Option<StreamEncoder>,
    out: Vec<u8>,
    pos: usize,
}

enum StreamEncoder {
    Brotli(Box<brotli::CompressorWriter<Vec<u8>>>),
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
}

impl StreamingEncoder {
    pub fn new(rx: std::sync::mpsc::Receiver<Vec<u8>>, encoding: Encoding) -> Self {
        let encoder = match encoding {
            // Quality 5 as for whole bodies; a 1 MB window is plenty for a page.
            Encoding::Brotli => StreamEncoder::Brotli(Box::new(brotli::CompressorWriter::new(
                Vec::new(),
                16 * 1024,
                5,
                20,
            ))),
            Encoding::Gzip => StreamEncoder::Gzip(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::new(6),
            )),
        };
        Self {
            rx,
            encoder: Some(encoder),
            out: Vec::new(),
            pos: 0,
        }
    }

    /// Compress one chunk and flush: the bytes produced so far.
    fn push(&mut self, chunk: &[u8]) -> std::io::Result<Vec<u8>> {
        match self.encoder.as_mut() {
            Some(StreamEncoder::Brotli(w)) => {
                w.write_all(chunk)?;
                w.flush()?;
                Ok(std::mem::take(w.get_mut()))
            }
            Some(StreamEncoder::Gzip(w)) => {
                w.write_all(chunk)?;
                // A sync flush: everything written so far is decodable.
                w.flush()?;
                Ok(std::mem::take(w.get_mut()))
            }
            None => Ok(Vec::new()),
        }
    }

    /// End the compressed stream: the trailer bytes.
    fn finish(&mut self) -> std::io::Result<Vec<u8>> {
        match self.encoder.take() {
            Some(StreamEncoder::Brotli(w)) => Ok(w.into_inner()),
            Some(StreamEncoder::Gzip(w)) => w.finish(),
            None => Ok(Vec::new()),
        }
    }
}

impl std::io::Read for StreamingEncoder {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.pos < self.out.len() {
                let n = (self.out.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.encoder.is_none() {
                return Ok(0);
            }
            self.out = match self.rx.recv() {
                Ok(chunk) if !chunk.is_empty() => self.push(&chunk)?,
                // An empty chunk or a closed channel ends the body.
                _ => self.finish()?,
            };
            self.pos = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn json_body() -> Vec<u8> {
        let rows: Vec<serde_json::Value> = (0..500)
            .map(|i| serde_json::json!({ "id": format!("p{i}"), "address": format!("{i} Main St"), "status": "lead" }))
            .collect();
        serde_json::to_vec(&serde_json::json!({ "changes": rows })).unwrap()
    }

    #[test]
    fn negotiation_prefers_brotli_then_gzip() {
        assert_eq!(negotiate(Some("gzip, deflate, br")), Some(Encoding::Brotli));
        assert_eq!(negotiate(Some("gzip")), Some(Encoding::Gzip));
        assert_eq!(negotiate(Some("br;q=0, gzip")), Some(Encoding::Gzip));
        assert_eq!(negotiate(Some("identity")), None);
        assert_eq!(negotiate(None), None);
        assert!(accepts_encoding("BR;q=0.5", "br"));
        assert!(!accepts_encoding("gzip", "br"));
        assert!(accepts_encoding("*", "br"));
        assert!(!accepts_encoding("*;q=0", "gzip"));
        // An explicit entry wins over the wildcard.
        assert!(!accepts_encoding("*, br;q=0", "br"));
        assert!(!accepts_encoding("", "gzip"));
    }

    #[test]
    fn compressed_bodies_round_trip() {
        let body = json_body();
        let br = compress(&body, Encoding::Brotli).unwrap();
        assert!(br.len() < body.len() / 4, "{} vs {}", br.len(), body.len());
        let mut out = Vec::new();
        brotli::Decompressor::new(&br[..], 4096)
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, body);

        let gz = compress(&body, Encoding::Gzip).unwrap();
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, body);
    }

    #[test]
    fn a_stream_is_flushed_chunk_by_chunk_and_decodes_whole() {
        for encoding in [Encoding::Gzip, Encoding::Brotli] {
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
            let mut body = StreamingEncoder::new(rx, encoding);
            let shell = b"<!DOCTYPE html><html><body><main>shell</main>".to_vec();
            tx.send(shell.clone()).unwrap();
            // The first chunk is readable on its own before the render
            // finishes: decode what has arrived so far.
            let mut first = vec![0u8; 64 * 1024];
            let n = body.read(&mut first).unwrap();
            assert!(n > 0, "{encoding:?}: nothing after the first flush");
            let mut partial = Vec::new();
            match encoding {
                Encoding::Gzip => {
                    let mut d = flate2::write::GzDecoder::new(Vec::new());
                    std::io::Write::write_all(&mut d, &first[..n]).unwrap();
                    std::io::Write::flush(&mut d).unwrap();
                    partial = d.get_ref().clone();
                }
                Encoding::Brotli => {
                    let mut d = brotli::DecompressorWriter::new(Vec::new(), 4096);
                    std::io::Write::write_all(&mut d, &first[..n]).unwrap();
                    std::io::Write::flush(&mut d).unwrap();
                    partial.extend_from_slice(d.get_ref());
                }
            }
            assert_eq!(
                partial, shell,
                "{encoding:?}: the shell decodes before the end"
            );

            let boundary = b"<template>resolved row</template></body></html>".to_vec();
            tx.send(boundary.clone()).unwrap();
            drop(tx);
            let mut rest = Vec::new();
            body.read_to_end(&mut rest).unwrap();
            let mut wire = first[..n].to_vec();
            wire.extend_from_slice(&rest);
            let mut whole = Vec::new();
            match encoding {
                Encoding::Gzip => flate2::read::GzDecoder::new(&wire[..])
                    .read_to_end(&mut whole)
                    .unwrap(),
                Encoding::Brotli => brotli::Decompressor::new(&wire[..], 4096)
                    .read_to_end(&mut whole)
                    .unwrap(),
            };
            assert_eq!(whole, [shell.clone(), boundary].concat(), "{encoding:?}");
        }
    }

    #[test]
    fn small_bodies_and_binary_types_are_left_alone() {
        assert_eq!(compress(b"{\"ok\":true}", Encoding::Gzip), None);
        assert!(compressible_content_type("application/json; charset=utf-8"));
        assert!(compressible_content_type("text/html"));
        assert!(!compressible_content_type("image/png"));
        assert!(!compressible_content_type("application/octet-stream"));
    }
}
