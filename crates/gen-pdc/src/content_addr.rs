//! The content-address SEAL — PDC clause 1, at the border.
//!
//! `/algorithmic-prowess-seal`: the best-fit construction for "every
//! buildable node carries a non-empty content address that is a pure
//! deterministic function of its own source bytes and the addresses of
//! its direct dependencies" is **content-addressing over a Merkle DAG**
//! (BLAKE3), with the address itself a **refined type** whose only
//! constructor rejects the empty/malformed case at the parse boundary.
//!
//! ## Why this is the seal, and its honest tier
//!
//! Today every ecosystem expresses clause 1 as a runtime `Violation`
//! (`gen-cargo::RegistryWithoutSha256`, `gen-gomod::NodeMissingSourceHash`)
//! — the address is a `String` / `Option<String>` on the border struct,
//! and "it is present + non-empty" is checked *after* construction. That
//! is **only-mitigated**: a spec with an empty address is *constructible*
//! and a runtime walk catches it.
//!
//! [`ContentAddr`] is the destination. Its field is private and its only
//! constructor (`try_parse`) rejects the empty/malformed case, so:
//!
//! - **In-Rust (library) construction of an empty address is
//!   truly-unrepresentable** — there is no expressible path; the type has
//!   no public field and no infallible constructor. (UNREP §III destination.)
//! - **At the wire (deserialize) boundary it is parse-time-rejected** —
//!   `Deserialize` routes through `try_parse`, so an empty address in JSON
//!   is an `Err` at the border, not a downstream `Violation`.
//!
//! This is the tier-promotion the PDC codification names: clause 1 moves
//! from *only-mitigated* (a `Vec<Violation>` row per ecosystem) to
//! *truly-unrepresentable (library) / parse-time-rejected (wire)* — once a
//! border adopts `ContentAddr` in place of its `String` hash field. gen-pdc
//! ships the type; the per-adapter border migration is the burn-down
//! (tracked, not done here — the adapter borders are owned by their crates,
//! and gen-gomod's is under active development).
//!
//! No ML. Pure content-addressing + a refinement type — the top of the
//! smallest-sufficient ladder for this invariant.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A sealed per-dependency content address (PDC clause 1).
///
/// Invariant, by construction: the inner string is **non-empty** and is a
/// lowercase hex BLAKE3 digest (64 hex chars) OR an accepted upstream
/// content-address form (a non-empty opaque token — e.g. a registry
/// `sha256` a variant sources verbatim). The `try_parse` constructor is
/// the *only* way in, so an empty address has no code path.
///
/// The field is private: `ContentAddr(String)` cannot be built by a struct
/// literal outside this module. That absence is the seal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentAddr(String);

/// Why a candidate string is not a valid content address. Surfaced at the
/// parse boundary (deserialize) and by `try_parse`.
///
/// The `WrongLength` / `NotHex` / `Genesis` variants back the stricter
/// [`ContentAddr::parse_digest`] constructor (absorbed from the standalone
/// supercache-contract codification), which enforces the canonical
/// 64-lowercase-hex BLAKE3 form so a cache key has exactly one spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentAddrError {
    /// The address is empty — clause 1's forbidden state.
    Empty,
    /// The address was not exactly 64 chars (a BLAKE3-256 digest is 64 hex).
    WrongLength(usize),
    /// The address contained a non-`[0-9a-f]` character — uppercase and
    /// non-hex are both rejected so the canonical form is unambiguous.
    NotHex(char),
    /// The address decoded to the all-zero genesis sentinel — a "not yet
    /// content-addressed" marker, never a valid per-dependency address.
    Genesis,
}

impl std::fmt::Display for ContentAddrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentAddrError::Empty => {
                write!(f, "content address is empty (PDC clause 1 forbids it)")
            }
            ContentAddrError::WrongLength(n) => {
                write!(f, "content address must be 64 hex chars, got {n}")
            }
            ContentAddrError::NotHex(c) => {
                write!(
                    f,
                    "content address contains a non-lowercase-hex character: {c:?}"
                )
            }
            ContentAddrError::Genesis => {
                write!(
                    f,
                    "content address is the all-zero genesis sentinel (no source hashed)"
                )
            }
        }
    }
}

impl std::error::Error for ContentAddrError {}

impl ContentAddr {
    /// The ONLY constructor. Rejects the empty case at the boundary.
    ///
    /// # Errors
    /// [`ContentAddrError::Empty`] when `s` is empty (or whitespace-only).
    pub fn try_parse(s: impl Into<String>) -> Result<Self, ContentAddrError> {
        let s = s.into();
        if s.trim().is_empty() {
            return Err(ContentAddrError::Empty);
        }
        Ok(ContentAddr(s))
    }

    /// The STRICT constructor: accept only the canonical 64-lowercase-hex
    /// BLAKE3-256 form, rejecting wrong-length, uppercase / non-hex, and the
    /// all-zero genesis sentinel at the boundary. (Absorbed from the
    /// standalone supercache-contract codification.)
    ///
    /// [`try_parse`](Self::try_parse) stays lenient by design — it accepts an
    /// opaque upstream content-address token a variant sources verbatim (a
    /// registry `sha256`, etc.). `parse_digest` is for the destination form
    /// where the address IS a BLAKE3 digest and a cache key must have exactly
    /// one spelling. An adapter that has moved its border to the digest form
    /// calls this; the merkle output already satisfies it.
    ///
    /// # Errors
    /// [`ContentAddrError::WrongLength`] / [`ContentAddrError::NotHex`] /
    /// [`ContentAddrError::Genesis`] when `s` is not a canonical digest.
    pub fn parse_digest(s: &str) -> Result<Self, ContentAddrError> {
        if s.len() != 64 {
            return Err(ContentAddrError::WrongLength(s.len()));
        }
        // Reject uppercase + non-hex so the canonical form is unambiguous.
        let mut all_zero = true;
        for c in s.chars() {
            match c {
                '1'..='9' => all_zero = false,
                'a'..='f' => all_zero = false,
                '0' => {}
                other => return Err(ContentAddrError::NotHex(other)),
            }
        }
        if all_zero {
            return Err(ContentAddrError::Genesis);
        }
        Ok(ContentAddr(s.to_string()))
    }

    /// Compute the Merkle content address of a node from its own source
    /// bytes and the (already-sealed) addresses of its direct dependencies.
    ///
    /// `addr(n) = BLAKE3( source(n) ‖ Σ addr(m) for n→m )`
    ///
    /// This IS PDC clause 1's defining function. Dependency addresses are
    /// sorted before folding so the address is order-independent (a set,
    /// not a sequence) — two specs that agree on the dependency *set*
    /// converge on the same address regardless of edge emission order.
    /// Deterministic, pure, no I/O.
    #[must_use]
    pub fn merkle(source: &[u8], mut dep_addrs: Vec<&ContentAddr>) -> Self {
        dep_addrs.sort();
        let mut h = blake3::Hasher::new();
        h.update(source);
        for d in dep_addrs {
            // length-prefix each child address so concatenation is
            // injective (no `ab|c` == `a|bc` collision across children).
            let bytes = d.0.as_bytes();
            h.update(&(bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        }
        // A BLAKE3 digest is 64 hex chars — always non-empty, so the
        // invariant holds by construction; skip the fallible path.
        ContentAddr(h.finalize().to_hex().to_string())
    }

    /// Borrow the sealed address as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ContentAddr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentAddr {
    /// Parse-time rejection: an empty address in the wire form is an `Err`
    /// at the boundary, never a value that flows downstream.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ContentAddr::try_parse(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_address_is_rejected_at_the_constructor() {
        // The seal: the ONLY constructor refuses the forbidden state.
        assert_eq!(ContentAddr::try_parse(""), Err(ContentAddrError::Empty));
        assert_eq!(ContentAddr::try_parse("   "), Err(ContentAddrError::Empty));
        assert!(ContentAddr::try_parse("deadbeef").is_ok());
    }

    #[test]
    fn empty_address_is_rejected_at_the_wire_boundary() {
        // parse-time-rejected: JSON `""` cannot deserialize into ContentAddr.
        let ok: Result<ContentAddr, _> = serde_json::from_str("\"deadbeef\"");
        assert!(ok.is_ok());
        let bad: Result<ContentAddr, _> = serde_json::from_str("\"\"");
        assert!(
            bad.is_err(),
            "empty content address must be rejected at deserialize"
        );
    }

    #[test]
    fn merkle_is_deterministic_and_dep_order_independent() {
        let a = ContentAddr::merkle(b"source-a", vec![]);
        let b = ContentAddr::merkle(b"source-b", vec![]);
        // same node, same deps → same address (pure).
        let n1 = ContentAddr::merkle(b"n", vec![&a, &b]);
        let n2 = ContentAddr::merkle(b"n", vec![&b, &a]); // deps reordered
        assert_eq!(
            n1, n2,
            "address must be a function of the dep SET, not order"
        );
        // different source → different address.
        let n3 = ContentAddr::merkle(b"n-prime", vec![&a, &b]);
        assert_ne!(n1, n3);
        // different dep set → different address (this is why editing a dep
        // re-derives the dependent — PDC clause 4, boundary-precise).
        let n4 = ContentAddr::merkle(b"n", vec![&a]);
        assert_ne!(n1, n4);
    }

    #[test]
    fn parse_digest_accepts_a_real_merkle_hex() {
        // The merkle output IS a canonical 64-lowercase-hex digest, so the
        // strict constructor round-trips it.
        let a = ContentAddr::merkle(b"payload", vec![]);
        let strict = ContentAddr::parse_digest(a.as_str()).expect("merkle hex is canonical");
        assert_eq!(a, strict);
        assert_eq!(a.as_str().len(), 64);
    }

    #[test]
    fn parse_digest_rejects_wrong_length() {
        assert_eq!(
            ContentAddr::parse_digest("abc"),
            Err(ContentAddrError::WrongLength(3))
        );
        assert!(matches!(
            ContentAddr::parse_digest(&"a".repeat(63)),
            Err(ContentAddrError::WrongLength(63))
        ));
    }

    #[test]
    fn parse_digest_rejects_uppercase_and_non_hex() {
        // uppercase is a non-canonical spelling of the same bytes — a cache
        // key must have exactly one form, so uppercase is rejected.
        let upper = "A".repeat(64);
        assert!(matches!(
            ContentAddr::parse_digest(&upper),
            Err(ContentAddrError::NotHex('A'))
        ));
        let bad = "z".repeat(64);
        assert!(matches!(
            ContentAddr::parse_digest(&bad),
            Err(ContentAddrError::NotHex('z'))
        ));
    }

    #[test]
    fn parse_digest_rejects_genesis_sentinel() {
        let genesis = "0".repeat(64);
        assert_eq!(
            ContentAddr::parse_digest(&genesis),
            Err(ContentAddrError::Genesis)
        );
    }

    #[test]
    fn length_prefix_makes_child_concat_injective() {
        // Without length-prefixing, {"ab","c"} and {"a","bc"} would collide.
        let ab = ContentAddr::try_parse("ab").unwrap();
        let c = ContentAddr::try_parse("c").unwrap();
        let a = ContentAddr::try_parse("a").unwrap();
        let bc = ContentAddr::try_parse("bc").unwrap();
        let left = ContentAddr::merkle(b"n", vec![&ab, &c]);
        let right = ContentAddr::merkle(b"n", vec![&a, &bc]);
        assert_ne!(left, right);
    }
}
