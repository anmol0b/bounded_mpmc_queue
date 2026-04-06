#[repr(C, align(64))]
pub struct CachePadded<T> {
    pub value: T,
    _pad: [u8; 56],
}

impl<T> CachePadded<T> {
    pub fn new(val: T) -> Self {
        Self {
            value: val,
            _pad: [0u8; 56],
        }
    }
}

impl<T> std::ops::Deref for CachePadded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}