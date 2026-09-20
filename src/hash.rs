//! Hash module: FNV-1a 64-bit implementation.

pub const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
pub const PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv1a(u64);

impl Fnv1a {
    pub fn new() -> Self {
        Self(OFFSET_BASIS)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let mut h = self.0;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
        self.0 = h;
    }

    pub fn finish(self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a whole buffer in one call. Kept for tests and for callers that
/// already hold the bytes; the copy core streams through `Fnv1a` instead.
#[allow(dead_code)]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = Fnv1a::new();
    h.write(bytes);
    h.finish()
}

#[cfg(test)]
#[path = "hash_tests.rs"]
mod tests;
