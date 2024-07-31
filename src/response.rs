use async_trait::async_trait;
use bytes::BytesMut;
use http_body_util::BodyExt;
use hyper::{
    body::{Body, Incoming},
    Response,
};

/// An extension to a hyper Response<Incoming> (returned by the Firecracker API socket) that allows
/// easy streaming of the response body.
#[async_trait]
pub trait HyperResponseExt {
    /// Stream the entire response body into a byte buffer (BytesMut).
    async fn recv_to_buf(&mut self) -> Result<BytesMut, hyper::Error>;

    /// Stream the entire response body into an owned string.
    async fn recv_to_string(&mut self) -> Result<String, hyper::Error> {
        let buf = self.recv_to_buf().await?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

#[async_trait]
impl HyperResponseExt for Response<Incoming> {
    async fn recv_to_buf(&mut self) -> Result<BytesMut, hyper::Error> {
        let mut buf = BytesMut::with_capacity(self.body().size_hint().lower() as usize);
        while let Some(frame) = self.frame().await {
            if let Ok(bytes) = frame?.into_data() {
                buf.extend(bytes);
            }
        }
        Ok(buf)
    }
}
