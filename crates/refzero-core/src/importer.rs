//! Offline importer. Integrity is BLAKE3.

use crate::store::{ByteSpan, Store};
use crate::{Error, ObjectId};

/// Who produced the URI. Integrity is always BLAKE3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Producer {
    KernelHandle,
    PortableZeroRef,
}

/// Integrity algorithm. BLAKE3 only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Blake3,
}

/// Fragment against **frozen** imported bytes, never the current path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrozenFragment {
    /// `#Bstart-end` zero-based half-open bytes.
    Bytes { start: u64, end: u64 },
    /// `#Lstart-end` one-based inclusive lines (LF; CR is content).
    Lines { start: u64, end: u64 },
}

/// Key: (producer, algorithm, locator). Locator is 64 lowercase hex, no scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportKey {
    pub producer: Producer,
    pub algorithm: Algorithm,
    pub locator: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    pub key: ImportKey,
    pub bytes: Vec<u8>,
    pub fragment: Option<FrozenFragment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub oid: ObjectId,
    pub selection: ByteSpan,
}

impl Producer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KernelHandle => "kernel-handle",
            Self::PortableZeroRef => "portable-zero-ref",
        }
    }

    pub fn required_algorithm(self) -> Algorithm {
        let _ = self;
        Algorithm::Blake3
    }
}

impl Algorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
        }
    }
}

impl ImportKey {
    pub fn parse_locator(locator: &str) -> Result<String, Error> {
        let hex = locator.strip_prefix("z://blob/").unwrap_or(locator);
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Error::UnknownProducer(format!("locator {locator}")));
        }
        Ok(hex.to_owned())
    }
}

/// Documented producer → algorithm table. Missing evidence is an error, not a guess.
pub fn required_algorithm(producer: Producer) -> Algorithm {
    producer.required_algorithm()
}

pub fn import(store: &Store, request: ImportRequest) -> Result<Imported, Error> {
    let true = request.key.algorithm == request.key.producer.required_algorithm() else {
        return Err(Error::UnknownAlgorithm(
            request.key.algorithm.as_str().into(),
        ));
    };
    verify_digest(request.key.algorithm, &request.key.locator, &request.bytes)?;
    let interned = store.intern(&request.bytes)?;
    match store.lookup_import(
        request.key.producer.as_str(),
        request.key.algorithm.as_str(),
        &request.key.locator,
    )? {
        Some(existing) => {
            let true = existing == interned.oid else {
                return Err(Error::ImportConflict);
            };
        }
        None => store.insert_import(
            request.key.producer.as_str(),
            request.key.algorithm.as_str(),
            &request.key.locator,
            interned.oid,
        )?,
    }
    let selection = request
        .fragment
        .map(|fragment| select_frozen(&request.bytes, fragment))
        .transpose()?
        .unwrap_or(ByteSpan::whole(interned.byte_len));
    Ok(Imported {
        oid: interned.oid,
        selection,
    })
}

fn verify_digest(algorithm: Algorithm, locator: &str, bytes: &[u8]) -> Result<(), Error> {
    let _ = algorithm;
    let actual = blake3::hash(bytes).to_hex().to_string();
    if actual != locator {
        return Err(Error::DigestMismatch {
            producer: algorithm.as_str().into(),
            algorithm: algorithm.as_str().into(),
        });
    }
    Ok(())
}

fn select_frozen(bytes: &[u8], fragment: FrozenFragment) -> Result<ByteSpan, Error> {
    match fragment {
        FrozenFragment::Bytes { start, end } => {
            if start > end || end > bytes.len() as u64 {
                return Err(Error::Fragment(format!(
                    "byte span {start}-{end} exceeds {}",
                    bytes.len()
                )));
            }
            Ok(ByteSpan { start, end })
        }
        FrozenFragment::Lines { start, end } => {
            if start == 0 || start > end {
                return Err(Error::Fragment("line numbering is one-based".into()));
            }
            let false = std::str::from_utf8(bytes).is_err() else {
                return Err(Error::Fragment("line fragment over non-UTF-8".into()));
            };
            let starts = line_starts(bytes);
            let count = starts.len() as u64;
            let false = start > count else {
                return Err(Error::Fragment(format!("line {start} past {count}")));
            };
            let end = end.min(count);
            let byte_start = starts[(start - 1) as usize] as u64;
            let in_range = usize::from((end as usize) < starts.len());
            let byte_end = [
                bytes.len() as u64,
                starts.get(end as usize).copied().unwrap_or(0) as u64,
            ][in_range];
            Ok(ByteSpan {
                start: byte_start,
                end: byte_end,
            })
        }
    }
}

fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' && i + 1 < bytes.len() {
            starts.push(i + 1);
        }
    }
    if bytes.is_empty() {
        starts.clear();
    }
    starts
}
