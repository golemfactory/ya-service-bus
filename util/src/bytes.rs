use crate::bytes::compat::CompatBytesMut;

pub trait BytesCompat<'a> {
    fn compat(self) -> compat::CompatBytesMut<'a>;
}

impl<'a> BytesCompat<'a> for &'a mut tokio_bytes::BytesMut {
    fn compat(self) -> CompatBytesMut<'a> {
        compat::bytes_mut(self)
    }
}

mod compat {
    use bytes;
    use tokio_bytes::{self, BufMut as _};

    pub struct CompatBytesMut<'a>(&'a mut tokio_bytes::BytesMut);

    impl<'a> bytes::BufMut for CompatBytesMut<'a> {
        #[inline]
        fn remaining_mut(&self) -> usize {
            self.0.remaining_mut()
        }

        #[inline]
        unsafe fn advance_mut(&mut self, cnt: usize) {
            self.0.advance_mut(cnt)
        }

        #[inline]
        unsafe fn bytes_mut(&mut self) -> &mut [u8] {
            std::mem::transmute(self.0.bytes_mut())
        }

        #[inline]
        fn put_slice(&mut self, src: &[u8]) {
            self.0.put_slice(src)
        }

        #[inline]
        fn put_u8(&mut self, n: u8) {
            self.0.put_u8(n)
        }

        #[inline]
        fn put_i8(&mut self, n: i8) {
            self.0.put_i8(n)
        }
    }

    pub fn bytes_mut(b: &mut tokio_bytes::BytesMut) -> CompatBytesMut {
        CompatBytesMut(b)
    }
}
