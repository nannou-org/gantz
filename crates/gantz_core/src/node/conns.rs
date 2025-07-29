//! A type that represents the connectedness of a node's inputs or outputs.
//!
//! Uses a fixed-size array of 256 bits, where each bit represents whether a
//! connection is connected.

use core::{convert, fmt, str::FromStr};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A bitset representing the connectivity state of up to 256 node connections.
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct Conns {
    /// The bits stored in an array of bytes.
    bytes: [u8; Self::MAX_BYTES],
    /// The node's actual number of inputs or outputs.
    len: usize,
}

/// The value or length is out of range of [`MAX`].
#[derive(Clone, Copy, Debug, Error, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[error("value out of bounds of max connections ({})", Conns::MAX)]
pub struct OutOfBoundsError;

/// Error type for parsing bit strings.
#[derive(Clone, Copy, Debug, Error, Eq, Hash, PartialEq, PartialOrd, Ord)]
#[error(
    "invalid bit string: must contain {} or less '0' and/or '1' chars",
    Conns::MAX
)]
pub struct ParseBitStringError;

impl Conns {
    /// The maximum number of inputs or outputs for a node.
    pub const MAX: usize = 256;

    /// The number of bytes used to represent the maximum number of bits.
    const MAX_BYTES: usize = Self::MAX / core::mem::size_of::<u8>();

    /// Creates a new `Conns` with the given number of connections.
    ///
    /// All connections are initialised as unconnected.
    ///
    /// Returns `Err` if `len` is out of range of [`MAX`].
    pub fn unconnected(len: usize) -> Result<Self, OutOfBoundsError> {
        if len > Self::MAX {
            Err(OutOfBoundsError)
        } else {
            Ok(Self {
                bytes: [0u8; Self::MAX_BYTES],
                len,
            })
        }
    }

    /// Creates a new `Conns` with the given number of connections.
    ///
    /// All connections are initialised as connected.
    ///
    /// Returns `Err` if `len` is out of range of [`MAX`].
    pub fn connected(len: usize) -> Result<Self, OutOfBoundsError> {
        let mut conns = Self::unconnected(len)?;
        for i in 0..len {
            conns.set(i, true).unwrap();
        }
        Ok(conns)
    }

    /// Creates a new `Conns` with the given slice of connection states.
    ///
    /// Returns `Err` if `len` is out of range of [`MAX`].
    pub fn try_from_slice(arr: &[bool]) -> Result<Self, OutOfBoundsError> {
        let mut conns = Self::unconnected(arr.len())?;
        for (i, &b) in arr.iter().enumerate() {
            conns.set(i, b).unwrap();
        }
        Ok(conns)
    }

    /// Creates a new `Conns` with the given iterator yielding connection states.
    ///
    /// Returns `Err` if `len` is out of range of [`MAX`].
    pub fn try_from_iter(iter: impl IntoIterator<Item = bool>) -> Result<Self, OutOfBoundsError> {
        let mut count = 0;
        let mut conns = Self::unconnected(Self::MAX).unwrap();
        for (i, b) in iter.into_iter().enumerate() {
            conns.set(i, b)?;
            count += 1;
        }
        conns.len = count;
        Ok(conns)
    }

    /// Gets the connection state at the given index.
    ///
    /// Returns `true` if connected, `false` if not.
    ///
    /// Returns `None` if the index is out of bounds.
    pub fn get(&self, i: usize) -> Option<bool> {
        if i >= self.len {
            return None;
        }
        let byte_index = i / 8;
        let bit_index = i % 8;
        Some((self.bytes[byte_index] >> bit_index) & 1 == 1)
    }

    /// Sets the connection state at the given index where:
    ///
    /// - `true` represents connected and
    /// - `false represents unconnected.
    ///
    /// Does nothing if the index is out of bounds.
    pub fn set(&mut self, i: usize, b: bool) -> Result<(), OutOfBoundsError> {
        if self.len <= i {
            return Err(OutOfBoundsError);
        }
        let byte_index = i / 8;
        let bit_index = i % 8;
        if b {
            self.bytes[byte_index] |= 1 << bit_index;
        } else {
            self.bytes[byte_index] &= !(1 << bit_index);
        }
        Ok(())
    }

    /// Returns an iterator over all connection states.
    pub fn iter(&self) -> impl Iterator<Item = bool> {
        (0..self.len).map(|i| self.get(i).unwrap())
    }

    /// Returns an iterator over all connection states represented as `1` or `0`.
    pub fn iter_bit_chars(&self) -> impl Iterator<Item = char> {
        self.iter().map(|b| if b { '1' } else { '0' })
    }

    /// The number of connections.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether or not there are no connections.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<'a> convert::TryFrom<&'a [bool]> for Conns {
    type Error = OutOfBoundsError;
    fn try_from(slice: &'a [bool]) -> Result<Self, Self::Error> {
        Self::try_from_slice(slice)
    }
}

impl<const N: usize> convert::TryFrom<[bool; N]> for Conns {
    type Error = OutOfBoundsError;
    fn try_from(arr: [bool; N]) -> Result<Self, Self::Error> {
        Self::try_from_slice(&arr)
    }
}

impl convert::TryFrom<Vec<bool>> for Conns {
    type Error = OutOfBoundsError;
    fn try_from(vec: Vec<bool>) -> Result<Self, Self::Error> {
        Self::try_from_slice(&vec)
    }
}

impl fmt::Debug for Conns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Conns({})", self)
    }
}

impl fmt::Display for Conns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.iter_bit_chars().collect::<String>())
    }
}

impl FromStr for Conns {
    type Err = ParseBitStringError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut conns = Self::unconnected(s.len()).map_err(|_| ParseBitStringError)?;
        for (i, ch) in s.chars().enumerate() {
            match ch {
                '1' => conns.set(i, true).unwrap(),
                '0' => (),
                _ => return Err(ParseBitStringError),
            }
        }
        Ok(conns)
    }
}

impl Serialize for Conns {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

impl<'de> Deserialize<'de> for Conns {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bitstring = String::deserialize(deserializer)?;
        Conns::from_str(&bitstring).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        Conns::unconnected(0).unwrap();
        Conns::unconnected(Conns::MAX).unwrap();
        Conns::unconnected(Conns::MAX + 1).unwrap_err();

        let conns = Conns::unconnected(10).unwrap();
        assert_eq!(conns.len, 10);
    }

    #[test]
    fn test_get() {
        let conns = Conns::unconnected(5).unwrap();

        // All bits should start as false
        assert_eq!(conns.get(0), Some(false));
        assert_eq!(conns.get(4), Some(false));

        // Out of bounds should return None
        assert_eq!(conns.get(5), None);
        assert_eq!(conns.get(100), None);
    }

    #[test]
    fn test_set() {
        let mut conns = Conns::unconnected(8).unwrap();

        // Set some bits
        conns.set(0, true).unwrap();
        conns.set(3, true).unwrap();
        conns.set(7, true).unwrap();

        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(3), Some(true));
        assert_eq!(conns.get(7), Some(true));

        // Set a bit back to false
        conns.set(0, false).unwrap();
        assert_eq!(conns.get(0), Some(false));

        // Out of bounds set should return error
        assert_eq!(conns.set(8, true), Err(OutOfBoundsError));
        assert_eq!(conns.get(8), None);
    }

    #[test]
    fn test_iter() {
        let mut conns = Conns::unconnected(4).unwrap();
        conns.set(0, true).unwrap();
        conns.set(2, true).unwrap();

        let collected: Vec<bool> = conns.iter().collect();
        assert_eq!(collected, vec![true, false, true, false]);
    }

    #[test]
    fn test_try_from_slice() {
        // Test successful creation from slice
        let slice = &[true, false, true, false, true];
        let conns = Conns::try_from_slice(slice).unwrap();
        assert_eq!(conns.len, 5);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));
        assert_eq!(conns.get(3), Some(false));
        assert_eq!(conns.get(4), Some(true));

        // Test empty slice
        let empty_slice = &[];
        let conns = Conns::try_from_slice(empty_slice).unwrap();
        assert_eq!(conns.len, 0);

        // Test slice that's too large
        let large_slice = vec![false; Conns::MAX + 1];
        assert_eq!(Conns::try_from_slice(&large_slice), Err(OutOfBoundsError));

        // Test maximum size slice
        let max_slice = vec![true; Conns::MAX];
        let conns = Conns::try_from_slice(&max_slice).unwrap();
        assert_eq!(conns.len, Conns::MAX);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(Conns::MAX - 1), Some(true));
    }

    #[test]
    fn test_try_from_iter() {
        // Test successful creation from iterator
        let vec = vec![true, false, true, false];
        let conns = Conns::try_from_iter(vec.into_iter()).unwrap();
        assert_eq!(conns.len, 4);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));
        assert_eq!(conns.get(3), Some(false));

        // Test empty iterator
        let empty_vec: Vec<bool> = vec![];
        let conns = Conns::try_from_iter(empty_vec.into_iter()).unwrap();
        assert_eq!(conns.len, 0);

        // Test iterator that's too large
        let large_iter = std::iter::repeat(false).take(Conns::MAX + 1);
        assert_eq!(Conns::try_from_iter(large_iter), Err(OutOfBoundsError));

        // Test maximum size iterator
        let max_iter = std::iter::repeat(true).take(Conns::MAX);
        let conns = Conns::try_from_iter(max_iter).unwrap();
        assert_eq!(conns.len, Conns::MAX);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(Conns::MAX - 1), Some(true));
    }

    #[test]
    fn test_try_from_array() {
        let arr = [true, false, true, false, true];
        let conns = Conns::try_from(arr).unwrap();
        assert_eq!(conns.len, 5);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));
        assert_eq!(conns.get(3), Some(false));
        assert_eq!(conns.get(4), Some(true));

        // Test empty array
        let empty_arr: [bool; 0] = [];
        let conns = Conns::try_from(empty_arr).unwrap();
        assert_eq!(conns.len, 0);
    }

    #[test]
    fn test_try_from_vec() {
        let vec = vec![true, false, true];
        let conns = Conns::try_from(vec).unwrap();
        assert_eq!(conns.len, 3);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));

        // Test error case
        let large_vec = vec![false; Conns::MAX + 1];
        assert_eq!(Conns::try_from(large_vec), Err(OutOfBoundsError));
    }

    #[test]
    fn test_debug_formatting() {
        // Test empty connections
        let conns = Conns::unconnected(0).unwrap();
        assert_eq!(format!("{:?}", conns), "Conns()");

        // Test some connections
        let mut conns = Conns::unconnected(6).unwrap();
        conns.set(0, false).unwrap();
        conns.set(1, false).unwrap();
        conns.set(2, true).unwrap();
        conns.set(3, false).unwrap();
        conns.set(4, true).unwrap();
        conns.set(5, true).unwrap();
        assert_eq!(format!("{:?}", conns), "Conns(001011)");

        // Test all true
        let mut conns = Conns::unconnected(4).unwrap();
        conns.set(0, true).unwrap();
        conns.set(1, true).unwrap();
        conns.set(2, true).unwrap();
        conns.set(3, true).unwrap();
        assert_eq!(format!("{:?}", conns), "Conns(1111)");

        // Test all false
        let conns = Conns::unconnected(3).unwrap();
        assert_eq!(format!("{:?}", conns), "Conns(000)");
    }

    #[test]
    fn test_cross_byte_boundaries() {
        // Test that bit operations work correctly across byte boundaries
        let mut conns = Conns::unconnected(16).unwrap();

        // Set bits in first byte
        conns.set(0, true).unwrap();
        conns.set(7, true).unwrap();

        // Set bits in second byte
        conns.set(8, true).unwrap();
        conns.set(15, true).unwrap();

        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(7), Some(true));
        assert_eq!(conns.get(8), Some(true));
        assert_eq!(conns.get(15), Some(true));

        // Verify other bits are false
        for i in [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14] {
            assert_eq!(conns.get(i), Some(false));
        }
    }

    #[test]
    fn test_iter_empty() {
        let conns = Conns::unconnected(0).unwrap();
        let collected: Vec<bool> = conns.iter().collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn test_iter_single() {
        let mut conns = Conns::unconnected(1).unwrap();
        conns.set(0, true).unwrap();
        let collected: Vec<bool> = conns.iter().collect();
        assert_eq!(collected, vec![true]);
    }
}

#[cfg(test)]
mod serde_tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_from_str() {
        // Test valid bit strings
        let conns = Conns::from_str("101").unwrap();
        assert_eq!(conns.len, 3);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));

        // Test empty string
        let conns = Conns::from_str("").unwrap();
        assert_eq!(conns.len, 0);

        // Test all zeros
        let conns = Conns::from_str("000").unwrap();
        assert_eq!(conns.len, 3);
        for i in 0..3 {
            assert_eq!(conns.get(i), Some(false));
        }

        // Test all ones
        let conns = Conns::from_str("111").unwrap();
        assert_eq!(conns.len, 3);
        for i in 0..3 {
            assert_eq!(conns.get(i), Some(true));
        }

        // Test invalid characters
        assert!(Conns::from_str("102").is_err());
        assert!(Conns::from_str("abc").is_err());
        assert!(Conns::from_str("1 0").is_err());

        // Test string that's too long
        let long_string = "1".repeat(Conns::MAX + 1);
        assert!(Conns::from_str(&long_string).is_err());
    }

    #[test]
    fn test_serialize() {
        // Test serializing various bit patterns
        let mut conns = Conns::unconnected(5).unwrap();
        conns.set(0, true).unwrap();
        conns.set(2, true).unwrap();
        conns.set(4, true).unwrap();

        let json = serde_json::to_string(&conns).unwrap();
        assert_eq!(json, "\"10101\"");

        // Test empty connections
        let conns = Conns::unconnected(0).unwrap();
        let json = serde_json::to_string(&conns).unwrap();
        assert_eq!(json, "\"\"");

        // Test all false
        let conns = Conns::unconnected(4).unwrap();
        let json = serde_json::to_string(&conns).unwrap();
        assert_eq!(json, "\"0000\"");
    }

    #[test]
    fn test_deserialize() {
        // Test deserializing various bit patterns
        let json = "\"10101\"";
        let conns: Conns = serde_json::from_str(json).unwrap();
        assert_eq!(conns.len, 5);
        assert_eq!(conns.get(0), Some(true));
        assert_eq!(conns.get(1), Some(false));
        assert_eq!(conns.get(2), Some(true));
        assert_eq!(conns.get(3), Some(false));
        assert_eq!(conns.get(4), Some(true));

        // Test empty string
        let json = "\"\"";
        let conns: Conns = serde_json::from_str(json).unwrap();
        assert_eq!(conns.len, 0);

        // Test invalid bit string
        let json = "\"102\"";
        assert!(serde_json::from_str::<Conns>(json).is_err());

        // Test string that's too long
        let long_bitstring = "1".repeat(Conns::MAX + 1);
        let json = format!("\"{}\"", long_bitstring);
        assert!(serde_json::from_str::<Conns>(&json).is_err());
    }

    #[test]
    fn test_roundtrip() {
        // Test that serialize -> deserialize produces the same result
        let mut original = Conns::unconnected(8).unwrap();
        original.set(0, true).unwrap();
        original.set(3, true).unwrap();
        original.set(7, true).unwrap();

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Conns = serde_json::from_str(&json).unwrap();

        // Compare by iterating through all bits
        let original_bits: Vec<bool> = original.iter().collect();
        let deserialized_bits: Vec<bool> = deserialized.iter().collect();
        assert_eq!(original_bits, deserialized_bits);
        assert_eq!(original.len, deserialized.len);
    }

    #[test]
    fn test_cross_byte_boundary_serde() {
        // Test serialization/deserialization across byte boundaries
        let mut conns = Conns::unconnected(16).unwrap();
        conns.set(0, true).unwrap();
        conns.set(7, true).unwrap();
        conns.set(8, true).unwrap();
        conns.set(15, true).unwrap();

        let json = serde_json::to_string(&conns).unwrap();
        let deserialized: Conns = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.get(0), Some(true));
        assert_eq!(deserialized.get(7), Some(true));
        assert_eq!(deserialized.get(8), Some(true));
        assert_eq!(deserialized.get(15), Some(true));

        // Verify other bits are false
        for i in [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14] {
            assert_eq!(deserialized.get(i), Some(false));
        }
    }
}
