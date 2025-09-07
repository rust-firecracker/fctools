use std::{ffi::OsString, path::PathBuf};

use super::resource::Resource;

pub mod command_modifier;
pub mod jailer;

/// Arguments that can be passed to the main VMM/"firecracker" binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmArguments {
    pub(crate) api_socket: VmmApiSocket,
    log_level: Option<VmmLogLevel>,
    show_log_origin: bool,
    log_module: Option<OsString>,
    show_log_level: bool,
    enable_boot_timer: bool,
    api_max_payload_bytes: Option<u32>,
    mmds_size_limit: Option<u32>,
    disable_seccomp_filter: bool,
    enable_pci_support: bool,
    log_resource: Option<Resource>,
    metadata_resource: Option<Resource>,
    metrics_resource: Option<Resource>,
    seccomp_filter_resource: Option<Resource>,
}

impl VmmArguments {
    /// Create new [VmmArguments] with the given [VmmApiSocket] configuration for the VMM's API socket.
    pub fn new(api_socket: VmmApiSocket) -> Self {
        Self {
            api_socket,
            log_level: None,
            show_log_origin: false,
            log_module: None,
            show_log_level: false,
            enable_boot_timer: false,
            api_max_payload_bytes: None,
            mmds_size_limit: None,
            disable_seccomp_filter: false,
            enable_pci_support: false,
            log_resource: None,
            metadata_resource: None,
            metrics_resource: None,
            seccomp_filter_resource: None,
        }
    }

    /// Set the [VmmLogLevel] for the [VmmArguments].
    pub fn log_level(mut self, log_level: VmmLogLevel) -> Self {
        self.log_level = Some(log_level);
        self
    }

    /// Enable the showing of the log level by the VMM.
    pub fn show_log_level(mut self) -> Self {
        self.show_log_level = true;
        self
    }

    /// Enable the showing of the log origin by the VMM.
    pub fn show_log_origin(mut self) -> Self {
        self.show_log_origin = true;
        self
    }

    /// Set the text representing the log module being filtered for by the VMM.
    pub fn log_module<M: Into<OsString>>(mut self, log_module: M) -> Self {
        self.log_module = Some(log_module.into());
        self
    }

    /// Enable the boot timer for the VMM.
    pub fn enable_boot_timer(mut self) -> Self {
        self.enable_boot_timer = true;
        self
    }

    /// Set the max size of HTTP request payloads in bytes for the VMM's API server.
    pub fn api_max_payload_bytes(mut self, amount: u32) -> Self {
        self.api_max_payload_bytes = Some(amount);
        self
    }

    /// Set the maximum size of the MMDS storage of the VMM, in bytes.
    pub fn mmds_size_limit(mut self, mmds_size_limit: u32) -> Self {
        self.mmds_size_limit = Some(mmds_size_limit);
        self
    }

    /// Customize the seccomp filter used by the VMM, disable it or set it back to the default one.
    pub fn seccomp_filter(mut self, seccomp_filter: VmmSeccompFilter) -> Self {
        match seccomp_filter {
            VmmSeccompFilter::Default => {
                self.disable_seccomp_filter = false;
                self.seccomp_filter_resource = None;
            }
            VmmSeccompFilter::Disabled => {
                self.disable_seccomp_filter = true;
                self.seccomp_filter_resource = None;
            }
            VmmSeccompFilter::Custom(resource) => {
                self.disable_seccomp_filter = false;
                self.seccomp_filter_resource = Some(resource);
            }
        }

        self
    }

    /// Specify the [Resource] pointing to the log file for the VMM.
    pub fn logs(mut self, logs: Resource) -> Self {
        self.log_resource = Some(logs);
        self
    }

    /// Specify the [Resource] pointing to the metadata file for the VMM.
    pub fn metadata(mut self, metadata: Resource) -> Self {
        self.metadata_resource = Some(metadata);
        self
    }

    /// Specify the [Resource] pointing to the metrics file for the VMM.
    pub fn metrics(mut self, metrics: Resource) -> Self {
        self.metrics_resource = Some(metrics);
        self
    }

    /// Enable PCIe support in the VMM.
    pub fn enable_pci_support(mut self) -> Self {
        self.enable_pci_support = true;
        self
    }

    /// Get an iterator over the references for all the resources embedded in these [VmmArguments].
    pub fn get_resources(&self) -> VmmArgumentResources<'_> {
        VmmArgumentResources {
            arguments: self,
            logs: self.log_resource.is_some(),
            metadata: self.metadata_resource.is_some(),
            metrics: self.metrics_resource.is_some(),
            seccomp_filter: self.seccomp_filter_resource.is_some(),
        }
    }

    /// Join these [VmmArguments] into a buffer of process arguments, using the given optional config path.
    /// This function assumes all resources inside this [VmmArguments] struct are initialized, otherwise a panic is
    /// emitted. The order in which the argument [OsString]s are inserted into the resulting [Vec] is not stable!
    pub fn join(&self, config_path: Option<PathBuf>) -> Vec<OsString> {
        let mut args = Vec::with_capacity(1);

        match self.api_socket {
            VmmApiSocket::Disabled => {
                args.push(OsString::from("--no-api"));
            }
            VmmApiSocket::Enabled(ref socket_path) => {
                args.push(OsString::from("--api-sock"));
                args.push(OsString::from(socket_path));
            }
        }

        if let Some(config_path) = config_path {
            args.push(OsString::from("--config-file"));
            args.push(OsString::from(config_path));
        }

        if let Some(log_level) = self.log_level {
            args.push(OsString::from("--level"));
            args.push(OsString::from(log_level.to_string()));
        }

        if self.show_log_origin {
            args.push(OsString::from("--show-log-origin"));
        }

        if let Some(module) = self.log_module.clone() {
            args.push(OsString::from("--module"));
            args.push(module);
        }

        if self.show_log_level {
            args.push(OsString::from("--show-level"));
        }

        if self.enable_boot_timer {
            args.push(OsString::from("--boot-timer"));
        }

        if let Some(max_payload) = self.api_max_payload_bytes {
            args.push(OsString::from("--http-api-max-payload-size"));
            args.push(OsString::from(max_payload.to_string()));
        }

        if let Some(limit) = self.mmds_size_limit {
            args.push(OsString::from("--mmds-size-limit"));
            args.push(OsString::from(limit.to_string()));
        }

        if self.disable_seccomp_filter {
            args.push(OsString::from("--no-seccomp"));
        } else if let Some(ref resource) = self.seccomp_filter_resource {
            args.push(OsString::from("--seccomp-filter"));
            args.push(self.get_resource_path(resource));
        }

        if let Some(ref resource) = self.log_resource {
            args.push(OsString::from("--log-path"));
            args.push(self.get_resource_path(resource));
        }

        if let Some(ref resource) = self.metadata_resource {
            args.push(OsString::from("--metadata"));
            args.push(self.get_resource_path(resource));
        }

        if let Some(ref resource) = self.metrics_resource {
            args.push(OsString::from("--metrics-path"));
            args.push(self.get_resource_path(resource));
        }

        if self.enable_pci_support {
            args.push(OsString::from("--enable-pci"));
        }

        args
    }

    #[inline(always)]
    fn get_resource_path(&self, resource: &Resource) -> OsString {
        resource
            .get_virtual_path()
            .expect("Resource is uninitialized at the time of argument join")
            .into()
    }
}

/// An iterator over the references of all resources embedded in an instance of [VmmArguments], with both
/// the iterator itself as well as its items being bound to the lifetime of the [VmmArguments].
pub struct VmmArgumentResources<'a> {
    arguments: &'a VmmArguments,
    logs: bool,
    metadata: bool,
    metrics: bool,
    seccomp_filter: bool,
}

impl<'a> Iterator for VmmArgumentResources<'a> {
    type Item = &'a Resource;

    fn next(&mut self) -> Option<Self::Item> {
        match self.logs {
            true => {
                self.logs = false;
                self.arguments.log_resource.as_ref()
            }
            false => match self.metadata {
                true => {
                    self.metadata = false;
                    self.arguments.metadata_resource.as_ref()
                }
                false => match self.metrics {
                    true => {
                        self.metrics = false;
                        self.arguments.metrics_resource.as_ref()
                    }
                    false => match self.seccomp_filter {
                        true => {
                            self.seccomp_filter = false;
                            self.arguments.seccomp_filter_resource.as_ref()
                        }
                        false => None,
                    },
                },
            },
        }
    }
}

/// A configuration of a VMM's API Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmmApiSocket {
    /// The socket should be disabled via --no-api argument.
    Disabled,
    /// The socket should be enabled at the given path via --api-sock argument.
    Enabled(PathBuf),
}

/// A configuration of a VMM's seccomp filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmmSeccompFilter {
    /// The default seccomp filter optimized for the current host by the Firecracker developers should be used.
    /// This is the default behavior, in which no additional arguments are passed.
    Default,
    /// Seccomp filtering and the default filter as an extension of it should be disabled, which is not
    /// recommended for most production environments. The --no-seccomp argument is passed in this case.
    Disabled,
    /// A custom-made seccomp filter pointed to by the given moved [Resource] should be used in place of the
    /// default. With this option, the --seccomp-filter argument is passed alongside the [Resource]'s virtual path.
    Custom(Resource),
}

/// A level of logging used by the VMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "vm", derive(serde::Serialize, serde::Deserialize))]
pub enum VmmLogLevel {
    /// No logging.
    Off,
    /// Logging all messages.
    Trace,
    /// Logging debug and higher-priority messages.
    Debug,
    /// Logging info and higher-priority messages.
    Info,
    /// Logging warnings and higher-priority messages.
    Warn,
    /// Logging errors and higher-priority messages.
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
    use std::{ffi::OsString, path::PathBuf};

    use uuid::Uuid;

    use crate::{
        process_spawner::DirectProcessSpawner,
        runtime::tokio::TokioRuntime,
        vmm::{
            arguments::VmmSeccompFilter,
            ownership::VmmOwnershipModel,
            resource::{CreatedResourceType, Resource, ResourceType, system::ResourceSystem},
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

    #[tokio::test]
    async fn log_path_can_be_set() {
        test_with_resource(|path, resource| {
            check_without_config(new().logs(resource), ["--log-path", path]);
        })
        .await;
    }

    #[tokio::test]
    async fn metadata_path_can_be_set() {
        test_with_resource(|path, resource| {
            check_without_config(new().metadata(resource), ["--metadata", path]);
        })
        .await;
    }

    #[tokio::test]
    async fn metrics_path_can_be_set() {
        test_with_resource(|path, resource| {
            check_without_config(new().metrics(resource), ["--metrics-path", path]);
        })
        .await;
    }

    #[test]
    fn mmds_size_limit_can_be_set() {
        check_without_config(new().mmds_size_limit(1000), ["--mmds-size-limit", "1000"]);
    }

    #[test]
    fn default_seccomp_filter_can_be_used_implicitly() {
        check_without_config(new(), ["!--no-seccomp"]);
    }

    #[test]
    fn default_seccomp_filter_can_be_used_explicitly() {
        check_without_config(new().seccomp_filter(VmmSeccompFilter::Default), ["!--no-seccomp"]);
    }

    #[test]
    fn seccomp_filter_can_be_disabled() {
        check_without_config(new().seccomp_filter(VmmSeccompFilter::Disabled), ["--no-seccomp"]);
    }

    #[tokio::test]
    async fn custom_seccomp_filter_can_be_used() {
        test_with_resource(|path, resource| {
            check_without_config(
                new().seccomp_filter(VmmSeccompFilter::Custom(resource)),
                ["--seccomp-filter", path],
            );
        })
        .await;
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

    #[test]
    fn pci_support_is_disabled_by_default() {
        check_without_config(new(), ["!--enable-pci"]);
    }

    #[test]
    fn pci_support_can_be_enabled() {
        check_without_config(new().enable_pci_support(), ["--enable-pci"]);
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
                assert!(!joined_args.contains(&OsString::from(matcher)));
            } else {
                assert!(joined_args.contains(&OsString::from(matcher)));
            }
        }
    }

    async fn test_with_resource<F: FnOnce(&str, Resource) -> ()>(function: F) {
        let mut resource_system = ResourceSystem::new(DirectProcessSpawner, TokioRuntime, VmmOwnershipModel::Shared);
        let path = format!("/tmp/{}", Uuid::new_v4());
        let resource = resource_system
            .create_resource(path.clone(), ResourceType::Created(CreatedResourceType::File))
            .unwrap();
        resource.start_initialization_with_same_path().unwrap();
        resource_system.synchronize().await.unwrap();

        function(&path, resource.clone());

        resource.start_disposal().unwrap();
        resource_system.synchronize().await.unwrap();
    }
}
