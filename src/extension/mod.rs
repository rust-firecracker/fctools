#[cfg(feature = "grpc-vsock-extension")]
pub mod grpc_vsock;

#[cfg(feature = "http-vsock-extension")]
pub mod http_vsock;

#[cfg(feature = "link-local-extension")]
pub mod link_local;

#[cfg(feature = "metrics-extension")]
pub mod metrics;

#[cfg(feature = "snapshot-editor-extension")]
pub mod snapshot_editor;
