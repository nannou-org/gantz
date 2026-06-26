//! A dedicated hash trait for constructing a content address.

/// The [`blake3`] hasher used for gantz' content addressing.
pub type Hasher = blake3::Hasher;

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

impl CaHash for bool {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(&[*self as u8]);
    }
}

impl<T: CaHash, const N: usize> CaHash for [T; N] {
    fn hash(&self, hasher: &mut Hasher) {
        // No length prefix: the element count is fixed by the type. For `[u8;
        // N]` this streams the same bytes as a single bulk update, so existing
        // content addresses are unchanged.
        for elem in self {
            elem.hash(hasher);
        }
    }
}

impl CaHash for str {
    fn hash(&self, hasher: &mut Hasher) {
        hasher.update(self.as_bytes());
    }
}

impl CaHash for String {
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

impl<T: CaHash> CaHash for Vec<T> {
    fn hash(&self, hasher: &mut Hasher) {
        (self.len() as u64).hash(hasher);
        for elem in self {
            elem.hash(hasher);
        }
    }
}

impl<T: CaHash> CaHash for std::collections::BTreeSet<T> {
    fn hash(&self, hasher: &mut Hasher) {
        (self.len() as u64).hash(hasher);
        for elem in self {
            elem.hash(hasher);
        }
    }
}

impl CaHash for crate::ContentAddr {
    fn hash(&self, hasher: &mut Hasher) {
        self.0.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use crate::content_addr;

    /// The generalized `[T; N]` impl must stream `[u8; N]` byte-for-byte the
    /// same as the slice impl, so promoting it doesn't shift existing content
    /// addresses.
    #[test]
    fn u8_array_matches_slice() {
        let arr: [u8; 3] = [1, 2, 3];
        let slice: &[u8] = &arr[..];
        assert_eq!(content_addr(&arr), content_addr(slice));
    }

    /// Non-`u8` arrays now hash element-wise and are order-sensitive.
    #[test]
    fn u16_array_is_element_and_order_sensitive() {
        assert_ne!(content_addr(&[1u16, 2]), content_addr(&[2u16, 1]));
        assert_eq!(content_addr(&[1u16, 2]), content_addr(&[1u16, 2]));
    }
}
