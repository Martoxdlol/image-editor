use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable actor identity for the CRDT-shaped op log (spec §3.1).
/// A single local actor exists today; the type is collaboration-ready.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct ActorId(pub u32);

/// Lamport-stamped operation id (spec §3.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
pub struct OpId {
    pub actor: ActorId,
    pub lamport: u64,
}

impl fmt::Display for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.actor.0, self.lamport)
    }
}

/// Undo grouping: one drag = one txn (spec §3.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct TxnId(pub u64);

/// Node identity. Encoded as `actor:counter` so ids minted by different
/// actors can never collide (paste always mints fresh ids, spec §10.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId {
    pub actor: u32,
    pub counter: u64,
}

impl NodeId {
    pub const fn new(actor: u32, counter: u64) -> Self {
        Self { actor, counter }
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.actor, self.counter)
    }
}

impl Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let (a, c) = s
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("NodeId must be actor:counter"))?;
        Ok(NodeId {
            actor: a.parse().map_err(serde::de::Error::custom)?,
            counter: c.parse().map_err(serde::de::Error::custom)?,
        })
    }
}

/// Content-address for binary blobs (tiles, images, fonts) — spec §3.2.
/// FNV-1a 64-bit over the payload; hex-encoded in JSON.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BlobHash(pub u64);

impl BlobHash {
    pub fn of(bytes: &[u8]) -> Self {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        BlobHash(h)
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl Serialize for BlobHash {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        u64::from_str_radix(&s, 16)
            .map(BlobHash)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_roundtrip() {
        let id = NodeId::new(7, 42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"7:42\"");
        assert_eq!(serde_json::from_str::<NodeId>(&json).unwrap(), id);
    }

    #[test]
    fn blob_hash_stable() {
        assert_eq!(BlobHash::of(b"hello"), BlobHash::of(b"hello"));
        assert_ne!(BlobHash::of(b"hello"), BlobHash::of(b"world"));
        let h = BlobHash::of(b"x");
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<BlobHash>(&json).unwrap(), h);
    }
}
