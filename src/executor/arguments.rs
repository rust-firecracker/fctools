use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Arguments passed by relevant executors to the "firecracker" binary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VmmArguments {
    // main
    pub(crate) api_socket: VmmApiSocket,
    config_path: Option<PathBuf>,
    // logging
    log_level: Option<VmmLogLevel>,
    pub(crate) log_path: Option<PathBuf>,
    show_log_origin: bool,
    log_module: Option<String>,
    show_log_level: bool,
    // misc
    enable_boot_timer: bool,
    api_max_payload_bytes: Option<u32>,
    metadata_path: Option<PathBuf>,
    pub(crate) metrics_path: Option<PathBuf>,
    mmds_size_limit: Option<u32>,
    disable_seccomp: bool,
    seccomp_path: Option<PathBuf>,
}

pub enum ConfigurationFileOverride {
    NoOverride,
    Disable,
    Enable(PathBuf),
}

impl VmmArguments {
    pub fn new(api_socket: VmmApiSocket) -> Self {
        Self {
            api_socket,
            config_path: None,
            log_level: None,
            log_path: None,
            show_log_origin: false,
            log_module: None,
            show_log_level: false,
            enable_boot_timer: false,
            api_max_payload_bytes: None,
            metadata_path: None,
            metrics_path: None,
            mmds_size_limit: None,
            disable_seccomp: false,
            seccomp_path: None,
        }
    }

    pub fn config_path(mut self, config_path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    pub fn log_level(mut self, log_level: VmmLogLevel) -> Self {
        self.log_level = Some(log_level);
        self
    }

    pub fn log_path(mut self, log_path: impl Into<PathBuf>) -> Self {
        self.log_path = Some(log_path.into());
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

    pub fn metadata_path(mut self, metadata_path: impl Into<PathBuf>) -> Self {
        self.metadata_path = Some(metadata_path.into());
        self
    }

    pub fn metrics_path(mut self, metrics_path: impl Into<PathBuf>) -> Self {
        self.metrics_path = Some(metrics_path.into());
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

    pub(crate) fn join(&self, config_override: ConfigurationFileOverride) -> Vec<String> {
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

        match config_override {
            ConfigurationFileOverride::NoOverride => {
                if let Some(ref config_path) = self.config_path {
                    args.push("--config-file".to_string());
                    args.push(config_path.to_string_lossy().into_owned());
                }
            }
            ConfigurationFileOverride::Disable => {}
            ConfigurationFileOverride::Enable(path) => {
                args.push("--config-file".to_string());
                args.push(path.to_string_lossy().into_owned());
            }
        }

        if let Some(log_level) = self.log_level {
            args.push("--level".to_string());
            args.push(log_level.to_string());
        }

        if let Some(ref log_path) = self.log_path {
            args.push("--log-path".to_string());
            args.push(log_path.to_string_lossy().into_owned());
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

        if let Some(ref metadata_path) = self.metadata_path {
            args.push("--metadata".to_string());
            args.push(metadata_path.to_string_lossy().into_owned());
        }

        if let Some(ref metrics_path) = self.metrics_path {
            args.push("--metrics-path".to_string());
            args.push(metrics_path.to_string_lossy().into_owned());
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

/// Arguments that are passed by relevant executors into the "jailer" binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JailerArguments {
    uid: u32,
    gid: u32,
    pub(crate) jail_id: String,

    cgroup_values: HashMap<String, String>,
    cgroup_version: Option<JailerCgroupVersion>,
    pub(crate) chroot_base_dir: Option<PathBuf>,
    daemonize: bool,
    network_namespace_path: Option<PathBuf>,
    exec_in_new_pid_ns: bool,
    parent_cgroup: Option<String>,
    resource_limits: HashMap<String, String>,
}

impl JailerArguments {
    pub fn new(uid: u32, gid: u32, jail_id: impl Into<String>) -> Self {
        Self {
            uid,
            gid,
            jail_id: jail_id.into(),
            cgroup_values: HashMap::new(),
            cgroup_version: None,
            chroot_base_dir: None,
            daemonize: false,
            network_namespace_path: None,
            exec_in_new_pid_ns: false,
            parent_cgroup: None,
            resource_limits: HashMap::new(),
        }
    }

    pub fn cgroup(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.cgroup_values.insert(key.into(), value.into());
        self
    }

    pub fn cgroups(mut self, cgroups: impl IntoIterator<Item = (String, String)>) -> Self {
        self.cgroup_values.extend(cgroups);
        self
    }

    pub fn cgroup_version(mut self, cgroup_version: JailerCgroupVersion) -> Self {
        self.cgroup_version = Some(cgroup_version);
        self
    }

    pub fn chroot_base_dir(mut self, chroot_base_dir: impl Into<PathBuf>) -> Self {
        self.chroot_base_dir = Some(chroot_base_dir.into());
        self
    }

    pub fn daemonize(mut self) -> Self {
        self.daemonize = true;
        self
    }

    pub fn network_namespace_path(mut self, network_namespace_path: impl Into<PathBuf>) -> Self {
        self.network_namespace_path = Some(network_namespace_path.into());
        self
    }

    pub fn exec_in_new_pid_ns(mut self) -> Self {
        self.exec_in_new_pid_ns = true;
        self
    }

    pub fn parent_cgroup(mut self, parent_cgroup: impl Into<String>) -> Self {
        self.parent_cgroup = Some(parent_cgroup.into());
        self
    }

    pub fn resource_limit(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.resource_limits.insert(key.into(), value.into());
        self
    }

    pub fn resource_limits(mut self, resource_limits: impl IntoIterator<Item = (String, String)>) -> Self {
        self.resource_limits.extend(resource_limits);
        self
    }

    pub(crate) fn join(&self, firecracker_binary_path: &Path) -> Vec<String> {
        let mut args = Vec::with_capacity(8);
        args.push("--exec-file".to_string());
        args.push(firecracker_binary_path.to_string_lossy().into_owned());
        args.push("--uid".to_string());
        args.push(self.uid.to_string());
        args.push("--gid".to_string());
        args.push(self.gid.to_string());
        args.push("--id".to_string());
        args.push(self.jail_id.to_string());

        if !self.cgroup_values.is_empty() {
            for (key, value) in &self.cgroup_values {
                args.push("--cgroup".to_string());
                args.push(format!("{key}={value}"));
            }
        }

        if let Some(cgroup_version) = self.cgroup_version {
            args.push("--cgroup-version".to_string());
            args.push(match cgroup_version {
                JailerCgroupVersion::V1 => "1".to_string(),
                JailerCgroupVersion::V2 => "2".to_string(),
            });
        }

        if let Some(ref chroot_base_dir) = self.chroot_base_dir {
            args.push("--chroot-base-dir".to_string());
            args.push(chroot_base_dir.to_string_lossy().into_owned());
        }

        if self.daemonize {
            args.push("--daemonize".to_string());
        }

        if let Some(ref network_namespace_path) = self.network_namespace_path {
            args.push("--netns".to_string());
            args.push(network_namespace_path.to_string_lossy().into_owned());
        }

        if self.exec_in_new_pid_ns {
            args.push("--new-pid-ns".to_string());
        }

        if let Some(parent_cgroup) = self.parent_cgroup.clone() {
            args.push("--parent-cgroup".to_string());
            args.push(parent_cgroup);
        }

        if !self.resource_limits.is_empty() {
            for (key, value) in &self.resource_limits {
                args.push("--resource-limit".to_string());
                args.push(format!("{key}={value}"));
            }
        }

        args
    }
}

/// A configuration of a Firecracker API Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmmApiSocket {
    /// The socket should be disabled via --no-api argument.
    Disabled,
    /// The socket should be enabled at the given path via --api-sock argument.
    Enabled(PathBuf),
}

/// A level of logging applied by Firecracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmmLogLevel {
    Off,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl ToString for VmmLogLevel {
    fn to_string(&self) -> String {
        match self {
            VmmLogLevel::Off => "Off",
            VmmLogLevel::Trace => "Trace",
            VmmLogLevel::Debug => "Debug",
            VmmLogLevel::Info => "Info",
            VmmLogLevel::Warn => "Warn",
            VmmLogLevel::Error => "Error",
        }
        .into()
    }
}

/// The cgroup version used by the jailer, v1 by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JailerCgroupVersion {
    /// Cgroups v1
    V1,
    /// Cgroups v2
    V2,
}

enum ArgValue<'a> {
    None,
    SomeBorrowed(&'a str),
    Some(String),
    Many(Vec<String>),
}

fn join_args(args: HashMap<String, ArgValue>) -> String {
    let mut output_str = String::new();

    for (key, arg_value) in args {
        output_str.push_str(
            match arg_value {
                ArgValue::None => format!("--{key}"),
                ArgValue::Some(value) => format!("--{key} {value}"),
                ArgValue::SomeBorrowed(value) => format!("--{key} {value}"),
                ArgValue::Many(values) => values
                    .into_iter()
                    .map(|value| format!("--{key} {value}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            }
            .as_str(),
        );

        output_str.push(' ');
    }

    output_str.pop();
    output_str
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ConfigurationFileOverride, JailerArguments, JailerCgroupVersion, VmmApiSocket, VmmArguments, VmmLogLevel,
    };

    #[test]
    fn firecracker_builds_correctly() {
        assert_fc(&["--no-api"], VmmArguments::new(VmmApiSocket::Disabled));
        assert_fc(
            &["--api-sock", "/tmp/socket.sock"],
            VmmArguments::new(VmmApiSocket::Enabled(PathBuf::from("/tmp/socket.sock"))),
        );
        assert_fc(
            &["--no-api", "--level Info"],
            VmmArguments::new(VmmApiSocket::Disabled).log_level(VmmLogLevel::Info),
        );
        assert_fc(
            &["--no-api", "--log-path /tmp/t.fifo"],
            VmmArguments::new(VmmApiSocket::Disabled).log_path("/tmp/t.fifo"),
        );
        assert_fc(
            &["--no-api", "--show-log-origin"],
            VmmArguments::new(VmmApiSocket::Disabled).show_log_origin(),
        );
        assert_fc(
            &["--no-api", "--module mod"],
            VmmArguments::new(VmmApiSocket::Disabled).log_module("mod"),
        );
        assert_fc(
            &["--no-api", "--show-level"],
            VmmArguments::new(VmmApiSocket::Disabled).show_log_level(),
        );
        assert_fc(
            &["--no-api", "--boot-timer"],
            VmmArguments::new(VmmApiSocket::Disabled).enable_boot_timer(),
        );
        assert_fc(
            &["--no-api", "--http-api-max-payload-size 32"],
            VmmArguments::new(VmmApiSocket::Disabled).api_max_payload_bytes(32),
        );
        assert_fc(
            &["--no-api", "--metadata /tmp/metadata"],
            VmmArguments::new(VmmApiSocket::Disabled).metadata_path("/tmp/metadata"),
        );
        assert_fc(
            &["--no-api", "--metrics-path /tmp/m.path"],
            VmmArguments::new(VmmApiSocket::Disabled).metrics_path("/tmp/m.path"),
        );
        assert_fc(
            &["--no-api", "--mmds-size-limit 128"],
            VmmArguments::new(VmmApiSocket::Disabled).mmds_size_limit(128),
        );
        assert_fc(
            &["--no-api", "--no-seccomp"],
            VmmArguments::new(VmmApiSocket::Disabled).disable_seccomp(),
        );
        assert_fc(
            &["--no-api", "--seccomp-filter /tmp/seccomp.filter"],
            VmmArguments::new(VmmApiSocket::Disabled).seccomp_path("/tmp/seccomp.filter"),
        );
    }

    #[test]
    fn jailer_builds_correctly() {
        let baseline = JailerArguments::new(1000, 1000, 1000.to_string());

        assert_jailer(
            &["--cgroup a=b", "--cgroup c=d"],
            baseline.clone().cgroup("a", "b").cgroup("c", "d"),
        );
        assert_jailer(
            &["--cgroup-version 1"],
            baseline.clone().cgroup_version(JailerCgroupVersion::V1),
        );
        assert_jailer(
            &["--cgroup-version 2"],
            baseline.clone().cgroup_version(JailerCgroupVersion::V2),
        );
        assert_jailer(
            &["--chroot-base-dir /tmp/chroot/dir"],
            baseline.clone().chroot_base_dir("/tmp/chroot/dir"),
        );
        assert_jailer(&["--daemonize"], baseline.clone().daemonize());
        assert_jailer(
            &["--netns /var/run/netns/testing"],
            baseline.clone().network_namespace_path("/var/run/netns/testing"),
        );
        assert_jailer(&["--new-pid-ns"], baseline.clone().exec_in_new_pid_ns());
        assert_jailer(&["--parent-cgroup cgroup"], baseline.clone().parent_cgroup("cgroup"));
        assert_jailer(
            &["--resource-limit a=b", "--resource-limit c=d"],
            baseline.clone().resource_limit("a", "b").resource_limit("c", "d"),
        );
    }

    fn assert_fc(expectations: &[&str], args: VmmArguments) {
        let joined_args = args.join(ConfigurationFileOverride::NoOverride);
        for expectation in expectations {
            assert!(joined_args.contains(expectation));
        }
    }

    fn assert_jailer(expectations: &[&str], args: JailerArguments) {
        let joined_args = args.join(&PathBuf::from("/tmp/fake/path"));
        for expectation in expectations {
            assert!(joined_args.contains(expectation));
        }
    }
}
