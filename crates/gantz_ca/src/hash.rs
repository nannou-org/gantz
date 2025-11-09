//! A dedicated hash trait for constructing a content address.

use crate::Hasher;

/// Types that can be hashed to produce a content address.
pub trait CaHash {
    /// Hash `self` to produce a stable content address.
    fn hash(&self, hasher: &mut Hasher);
}

impl CaHash for u8 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for u16 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for u32 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for u64 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for usize {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for i8 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for i16 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for i32 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for i64 {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl CaHash for isize {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self.to_be_bytes());
    }
}

impl<const N: usize> CaHash for [u8; N] {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self[..]);
    }
}

impl CaHash for str {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(self.as_bytes());
    }
}

impl CaHash for [u8] {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&self[..]);
    }
}

impl<T> CaHash for Box<T>
where
    T: ?Sized + CaHash,
{
    fn hash(&self, hasher: &mut Hasher) {
        (**self).hash(hasher);
    }
}

impl<T> CaHash for std::rc::Rc<T>
where
    T: ?Sized + CaHash,
{
    fn hash(&self, hasher: &mut Hasher) {
        (**self).hash(hasher);
    }
}

impl<T> CaHash for std::sync::Arc<T>
where
    T: ?Sized + CaHash,
{
    fn hash(&self, hasher: &mut Hasher) {
        (**self).hash(hasher);
    }
}

impl<T> CaHash for Option<T>
where
    T: CaHash,
{
    fn hash(&self, hasher: &mut Hasher) {
        const NONE: u8 = 0;
        const SOME: u8 = 1;
        match self {
            None => {
                hasher.update(&[NONE]);
            }
            Some(t) => {
                hasher.update(&[SOME]);
                t.hash(hasher);
            }
        }
    }
}

impl CaHash for crate::ContentAddr {
    fn hash(&self, hasher: &mut Hasher) {
        self.0.hash(hasher);
    }
}
