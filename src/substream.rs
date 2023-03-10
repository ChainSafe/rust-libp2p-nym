use futures::{
    io::{Error as IoError, ErrorKind, IoSlice, IoSliceMut},
    AsyncRead, AsyncWrite,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Debug)]
pub struct Substream {
    pub(crate) inbound_rx: UnboundedReceiver<Vec<u8>>,
    outbound_tx: UnboundedSender<Vec<u8>>,
}

impl Substream {
    pub fn new(
        inbound_rx: UnboundedReceiver<Vec<u8>>,
        outbound_tx: UnboundedSender<Vec<u8>>,
    ) -> Self {
        Substream {
            inbound_rx,
            outbound_tx,
        }
    }
}

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
