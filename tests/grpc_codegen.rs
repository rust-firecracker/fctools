// Recursive expansion of include_proto! macro
// ============================================

#[derive(Clone, Copy, PartialEq, ::prost::Message)]
pub struct Ping {
    #[prost(uint32, tag = "1")]
    pub number: u32,
}
#[derive(Clone, Copy, PartialEq, ::prost::Message)]
pub struct Pong {
    #[prost(uint32, tag = "1")]
    pub number: u32,
}
#[doc = " Generated client implementations."]
pub mod guest_agent_service_client {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value
    )]
    use tonic::codegen::http::Uri;
    use tonic::codegen::*;
    #[derive(Debug, Clone)]
    pub struct GuestAgentServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl GuestAgentServiceClient<tonic::transport::Channel> {
        #[doc = " Attempt to create a new client by connecting to a given endpoint."]
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> GuestAgentServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::BoxBody>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(inner: T, interceptor: F) -> GuestAgentServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::BoxBody>,
                Response = http::Response<<T as tonic::client::GrpcService<tonic::body::BoxBody>>::ResponseBody>,
            >,
            <T as tonic::codegen::Service<http::Request<tonic::body::BoxBody>>>::Error:
                Into<StdError> + std::marker::Send + std::marker::Sync,
        {
            GuestAgentServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        #[doc = " Compress requests with the given encoding."]
        #[doc = ""]
        #[doc = " This requires the server to support it otherwise it might respond with an"]
        #[doc = " error."]
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        #[doc = " Enable decompressing responses."]
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        #[doc = " Limits the maximum size of a decoded message."]
        #[doc = ""]
        #[doc = " Default: `4MB`"]
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        #[doc = " Limits the maximum size of an encoded message."]
        #[doc = ""]
        #[doc = " Default: `usize::MAX`"]
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        pub async fn unary(
            &mut self,
            request: impl tonic::IntoRequest<super::Ping>,
        ) -> std::result::Result<tonic::Response<super::Pong>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/Unary");
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "Unary"));
            self.inner.unary(req, path, codec).await
        }
        pub async fn client_streaming(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::Ping>,
        ) -> std::result::Result<tonic::Response<super::Pong>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/ClientStreaming");
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "ClientStreaming"));
            self.inner.client_streaming(req, path, codec).await
        }
        pub async fn server_streaming(
            &mut self,
            request: impl tonic::IntoRequest<super::Ping>,
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<super::Pong>>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/ServerStreaming");
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "ServerStreaming"));
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn duplex_streaming(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::Ping>,
        ) -> std::result::Result<tonic::Response<tonic::codec::Streaming<super::Pong>>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| tonic::Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let codec = tonic::codec::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static("/guest_agent.GuestAgentService/DuplexStreaming");
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(GrpcMethod::new("guest_agent.GuestAgentService", "DuplexStreaming"));
            self.inner.streaming(req, path, codec).await
        }
    }
}
