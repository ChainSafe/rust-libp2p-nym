use futures::{
    io::{Error as IoError, ErrorKind, IoSlice, IoSliceMut},
    AsyncRead, AsyncWrite,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pub struct Substream {}

impl AsyncRead for Substream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
    ) -> Poll<Result<usize, IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }
}

impl AsyncWrite for Substream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, IoError>> {
        Poll::Ready(Err(IoError::new(ErrorKind::Other, "unimplemented")))
    }
}
