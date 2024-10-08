#[cfg(feature = "fcnet-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "fcnet-ext")))]
pub mod fcnet;

#[cfg(feature = "grpc-vsock-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc-vsock-ext")))]
pub mod grpc_vsock;

#[cfg(feature = "http-vsock-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "http-vsock-ext")))]
pub mod http_vsock;

#[cfg(feature = "link-local-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "link-local-ext")))]
pub mod link_local;

#[cfg(feature = "metrics-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics-ext")))]
pub mod metrics;

#[cfg(feature = "snapshot-editor-ext")]
#[cfg_attr(docsrs, doc(cfg(feature = "snapshot-editor-ext")))]
pub mod snapshot_editor;
