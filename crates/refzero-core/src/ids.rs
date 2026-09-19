use std::fmt;

use crate::Error;

const UUID_LEN: usize = 16;

/// Private allocated object identity (UUIDv4). Never a content digest.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId([u8; UUID_LEN]);

/// Session identity (UUIDv4). Owns the loc serial counter.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId([u8; UUID_LEN]);

impl ObjectId {
    pub(crate) fn new_v4() -> Result<Self, Error> {
        Ok(Self(uuid_v4()))
    }

    pub fn as_bytes(&self) -> &[u8; UUID_LEN] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; UUID_LEN]) -> Self {
        Self(bytes)
    }
}

impl SessionId {
    pub(crate) fn new_v4() -> Result<Self, Error> {
        Ok(Self(uuid_v4()))
    }

    pub fn as_bytes(&self) -> &[u8; UUID_LEN] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; UUID_LEN]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", hex(&self.0))
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({})", hex(&self.0))
    }
}

// Same RFC 4122 v4 bytes as the reference `getrandom` fill; uuid is already
// a workspace dependency, so no second CSPRNG crate is introduced.
fn uuid_v4() -> [u8; UUID_LEN] {
    *uuid::Uuid::new_v4().as_bytes()
}

pub(crate) fn hex(bytes: &[u8; UUID_LEN]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn blob_to_id(bytes: &[u8]) -> Result<[u8; UUID_LEN], Error> {
    <[u8; UUID_LEN]>::try_from(bytes).map_err(|_| Error::UnknownObject)
}
