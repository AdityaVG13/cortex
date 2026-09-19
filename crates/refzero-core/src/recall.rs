//! Cortex-free recall envelope. Round-trips interned object identity only.
//! Does not grant Edit. Cortex is a separate product and is not imported.

use serde::{Deserialize, Serialize};

use crate::ids::hex;
use crate::store::ByteSpan;
use crate::{Error, ObjectId};

/// Portable recall of an interned object. Fresh loc on the receiving session
/// is minted by the host; this envelope never carries a grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallEnvelope {
    pub oid_hex: String,
    pub start: u64,
    pub end: u64,
}

impl RecallEnvelope {
    pub fn from_oid(oid: ObjectId, span: ByteSpan) -> Self {
        Self {
            oid_hex: hex(oid.as_bytes()),
            start: span.start,
            end: span.end,
        }
    }

    pub fn oid(&self) -> Result<ObjectId, Error> {
        if self.oid_hex.len() != 32 {
            return Err(Error::UnknownObject);
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&self.oid_hex[i * 2..i * 2 + 2], 16)
                .map_err(|_| Error::UnknownObject)?;
        }
        Ok(ObjectId::from_bytes(bytes))
    }

    pub fn span(&self) -> ByteSpan {
        ByteSpan {
            start: self.start,
            end: self.end,
        }
    }
}
