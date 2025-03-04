use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};

use crate::docker_mgr::{DockerMessage, DockerMessageType};

pub struct AsyncRWTracker<T> {
    sender: mpsc::Sender<DockerMessage>,
    inner: T,
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncRWTracker<T> {
    pub fn new(sender: mpsc::Sender<DockerMessage>, inner: T) -> Self {
        Self { sender, inner }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for AsyncRWTracker<T> {
    fn poll_read(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> core::task::Poll<std::io::Result<()>> {
        use core::pin::Pin;
        use core::task::Poll;
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(data)) => {
                let _ = self.sender.try_send(DockerMessage {
                    message_type: DockerMessageType::ContainerPoke,
                    reply_to: None,
                });
                Poll::Ready(Ok(data))
            }
            x => x,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for AsyncRWTracker<T> {
    fn poll_write(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
        buf: &[u8],
    ) -> core::task::Poll<std::io::Result<usize>> {
        use core::pin::Pin;
        use core::task::Poll;
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(data)) => {
                let _ = self.sender.try_send(DockerMessage {
                    message_type: DockerMessageType::ContainerPoke,
                    reply_to: None,
                });
                Poll::Ready(Ok(data))
            }
            x => x,
        }
    }

    fn poll_flush(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<std::io::Result<()>> {
        use core::pin::Pin;
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<std::io::Result<()>> {
        use core::pin::Pin;
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
