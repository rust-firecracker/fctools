use std::path::PathBuf;

use super::resource::{created::CreatedVmmResource, moved::MovedVmmResource};

pub mod command_modifier;
pub mod jailer;

/// Arguments that can be passed to the main VMM/"firecracker" binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmArguments {
    // main
    pub(crate) api_socket: VmmApiSocket,
    // logging
    log_level: Option<VmmLogLevel>,
    pub(crate) logs: Option<CreatedVmmResource>,
    show_log_origin: bool,
    log_module: Option<String>,
    show_log_level: bool,
    // misc
    enable_boot_timer: bool,
    api_max_payload_bytes: Option<u32>,
    metadata: Option<MovedVmmResource>,
    pub(crate) metrics: Option<CreatedVmmResource>,
    mmds_size_limit: Option<u32>,
    disable_seccomp: bool,
    seccomp_path: Option<PathBuf>,
}

impl VmmArguments {
    pub fn new(api_socket: VmmApiSocket) -> Self {
        Self {
            api_socket,
            log_level: None,
            logs: None,
            show_log_origin: false,
            log_module: None,
            show_log_level: false,
            enable_boot_timer: false,
            api_max_payload_bytes: None,
            metadata: None,
            metrics: None,
            mmds_size_limit: None,
            disable_seccomp: false,
            seccomp_path: None,
        }
    }

    pub fn log_level(mut self, log_level: VmmLogLevel) -> Self {
        self.log_level = Some(log_level);
        self
    }

    pub fn logs(mut self, logs: CreatedVmmResource) -> Self {
        self.logs = Some(logs);
        self
    }

    pub fn show_log_level(mut self) -> Self {
        self.show_log_level = true;
        self
    }

    pub fn show_log_origin(mut self) -> Self {
        self.show_log_origin = true;
        self
    }

    pub fn log_module(mut self, log_module: impl Into<String>) -> Self {
        self.log_module = Some(log_module.into());
        self
    }

    pub fn enable_boot_timer(mut self) -> Self {
        self.enable_boot_timer = true;
        self
    }

    pub fn api_max_payload_bytes(mut self, amount: u32) -> Self {
        self.api_max_payload_bytes = Some(amount);
        self
    }

    pub fn metadata(mut self, metadata: MovedVmmResource) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn metrics(mut self, metrics: CreatedVmmResource) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn mmds_size_limit(mut self, mmds_size_limit: u32) -> Self {
        self.mmds_size_limit = Some(mmds_size_limit);
        self
    }

    pub fn disable_seccomp(mut self) -> Self {
        self.disable_seccomp = true;
        self
    }

    pub fn seccomp_path(mut self, seccomp_path: impl Into<PathBuf>) -> Self {
        self.seccomp_path = Some(seccomp_path.into());
        self
    }

    /// Join these [VmmArguments] into a [Vec] of process arguments, using the given optional config path.
    pub fn join(&self, config_path: Option<PathBuf>) -> Vec<String> {
        let mut args = Vec::with_capacity(1);

        match self.api_socket {
            VmmApiSocket::Disabled => {
                args.push("--no-api".to_string());
            }
            VmmApiSocket::Enabled(ref socket_path) => {
                args.push("--api-sock".to_string());
                args.push(socket_path.to_string_lossy().into_owned());
            }
        }

        if let Some(config_path) = config_path {
            args.push("--config-file".to_string());
            args.push(config_path.to_string_lossy().into_owned());
        }

        if let Some(log_level) = self.log_level {
            args.push("--level".to_string());
            args.push(log_level.to_string());
        }

        if let Some(ref log_path) = self.logs {
            args.push("--log-path".to_string());
            args.push(log_path.local_path().to_string_lossy().into_owned());
        }

        if self.show_log_origin {
            args.push("--show-log-origin".to_string());
        }

        if let Some(module) = self.log_module.clone() {
            args.push("--module".to_string());
            args.push(module);
        }

        if self.show_log_level {
            args.push("--show-level".to_string());
        }

        if self.enable_boot_timer {
            args.push("--boot-timer".to_string());
        }

        if let Some(max_payload) = self.api_max_payload_bytes {
            args.push("--http-api-max-payload-size".to_string());
            args.push(max_payload.to_string());
        }

        if let Some(ref metadata) = self.metadata {
            args.push("--metadata".to_string());
            args.push(metadata.local_path().to_string_lossy().into_owned());
        }

        if let Some(ref metrics_path) = self.metrics {
            args.push("--metrics-path".to_string());
            args.push(metrics_path.local_path().to_string_lossy().into_owned());
        }

        if let Some(ref limit) = self.mmds_size_limit {
            args.push("--mmds-size-limit".to_string());
            args.push(limit.to_string());
        }

        if self.disable_seccomp {
            args.push("--no-seccomp".to_string());
        }

        if let Some(ref seccomp_path) = self.seccomp_path {
            args.push("--seccomp-filter".to_string());
            args.push(seccomp_path.to_string_lossy().into_owned());
        }

        args
    }
}

/// A configuration of a VMM API Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmmApiSocket {
    /// The socket should be disabled via --no-api argument.
    Disabled,
    /// The socket should be enabled at the given path via --api-sock argument.
    Enabled(PathBuf),
}

/// A level of logging applied by the VMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "vm", derive(serde::Serialize, serde::Deserialize))]
pub enum VmmLogLevel {
    Off,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for VmmLogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmmLogLevel::Off => write!(f, "Off"),
            VmmLogLevel::Trace => write!(f, "Trace"),
            VmmLogLevel::Debug => write!(f, "Debug"),
            VmmLogLevel::Info => write!(f, "Info"),
            VmmLogLevel::Warn => write!(f, "Warn"),
            VmmLogLevel::Error => write!(f, "Error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        process_spawner::DirectProcessSpawner,
        runtime::tokio::TokioRuntime,
        vmm::{
            ownership::VmmOwnershipModel,
            resource::{
                created::{CreatedVmmResource, CreatedVmmResourceType},
                moved::{MovedVmmResource, VmmResourceMoveMethod},
            },
        },
    };

    use super::{VmmApiSocket, VmmArguments, VmmLogLevel};

    fn new() -> VmmArguments {
        VmmArguments::new(VmmApiSocket::Enabled(PathBuf::from("/tmp/api.sock")))
    }

    #[test]
    fn api_sock_can_be_disabled() {
        check_without_config(VmmArguments::new(VmmApiSocket::Disabled), ["--no-api"]);
    }

    #[test]
    fn api_sock_can_be_enabled() {
        check_without_config(new(), ["--api-sock", "/tmp/api.sock"]);
    }

    #[test]
    fn log_level_can_be_set() {
        check_without_config(new().log_level(VmmLogLevel::Error), ["--level", "Error"]);
    }

    #[test]
    fn log_path_can_be_set() {
        check_without_config(
            new().logs(CreatedVmmResource::new(
                "/tmp/some_logs.txt",
                CreatedVmmResourceType::File,
            )),
            ["--log-path", "/tmp/some_logs.txt"],
        );
    }

    #[test]
    fn show_log_origin_can_be_enabled() {
        check_without_config(new().show_log_origin(), ["--show-log-origin"]);
    }

    #[test]
    fn module_can_be_set() {
        check_without_config(new().log_module("some_module"), ["--module", "some_module"]);
    }

    #[test]
    fn show_log_level_can_be_enabled() {
        check_without_config(new().show_log_level(), ["--show-level"]);
    }

    #[test]
    fn boot_timer_can_be_enabled() {
        check_without_config(new().enable_boot_timer(), ["--boot-timer"]);
    }

    #[test]
    fn max_payload_can_be_set() {
        check_without_config(
            new().api_max_payload_bytes(1000),
            ["--http-api-max-payload-size", "1000"],
        );
    }

    #[test]
    fn metadata_path_can_be_set() {
        let mut resource = MovedVmmResource::new("/tmp/metadata.txt", VmmResourceMoveMethod::Rename);
        let _ = resource.initialize_with_same_path(VmmOwnershipModel::Shared, DirectProcessSpawner, TokioRuntime);
        check_without_config(new().metadata(resource), ["--metadata", "/tmp/metadata.txt"]);
    }

    #[test]
    fn metrics_path_can_be_set() {
        check_without_config(
            new().metrics(CreatedVmmResource::new(
                "/tmp/metrics.txt",
                CreatedVmmResourceType::File,
            )),
            ["--metrics-path", "/tmp/metrics.txt"],
        );
    }

    #[test]
    fn mmds_size_limit_can_be_set() {
        check_without_config(new().mmds_size_limit(1000), ["--mmds-size-limit", "1000"]);
    }

    #[test]
    fn seccomp_can_be_disabled() {
        check_without_config(new().disable_seccomp(), ["--no-seccomp"]);
    }

    #[test]
    fn seccomp_path_can_be_set() {
        check_without_config(new().seccomp_path("/tmp/seccomp"), ["--seccomp-filter", "/tmp/seccomp"]);
    }

    #[test]
    fn config_path_gets_added() {
        check_with_config(
            new(),
            Some("/tmp/override_config.json".into()),
            ["--config-file", "/tmp/override_config.json"],
        );
    }

    #[test]
    fn config_path_does_not_get_added() {
        check_with_config(new(), None, ["!--config-file", "!/tmp/config.json"]);
    }

    #[inline]
    fn check_without_config<const AMOUNT: usize>(args: VmmArguments, matchers: [&str; AMOUNT]) {
        check_with_config(args, None, matchers);
    }

    #[inline]
    fn check_with_config<const AMOUNT: usize>(
        args: VmmArguments,
        config_path: Option<PathBuf>,
        matchers: [&str; AMOUNT],
    ) {
        let joined_args = args.join(config_path);

        for matcher in matchers {
            if let Some(matcher) = matcher.strip_prefix("!") {
                assert!(!joined_args.contains(&matcher.to_string()));
            } else {
                assert!(joined_args.contains(&matcher.to_string()));
            }
        }
    }
}
