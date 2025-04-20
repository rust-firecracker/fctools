//! A set of extensions to the rest of fctools' functionality. These currently include:
//! - `grpc-vsock-extension`, allows gRPC connections to VMs via the tonic and tower crates.
//! - `http-vsock-extension`, allows HTTP connections to VMs (including connection pooling) via the hyper and hyper-util crates.
//! - `link-local-extension`, performs sequential IPAM for IPv4 subnets in the link-local range (169.254.0.0) by doing the needed math internally.
//! - `metrics-extension`, maps out the entire format of Firecracker's metrics to be used with [serde], and provides a task that can collect these metrics.
//! - `snapshot-editor-extension`, abstracts away the CLI interface of the "snapshot-editor" behind a typed interface that runs the process asynchronously.

#[cfg(feature = "grpc-vsock-extension")]
#[cfg_attr(docsrs, doc(cfg(feature = "grpc-vsock-extension")))]
pub mod grpc_vsock;

#[cfg(feature = "http-vsock-extension")]
#[cfg_attr(docsrs, doc(cfg(feature = "http-vsock-extension")))]
pub mod http_vsock;

#[cfg(feature = "link-local-extension")]
#[cfg_attr(docsrs, doc(cfg(feature = "link-local-extension")))]
pub mod link_local;

#[cfg(feature = "metrics-extension")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics-extension")))]
pub mod metrics;

#[cfg(feature = "snapshot-editor-extension")]
#[cfg_attr(docsrs, doc(cfg(feature = "snapshot-editor-extension")))]
pub mod snapshot_editor;
