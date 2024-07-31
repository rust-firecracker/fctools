use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Arguments passed by relevant executors to the "firecracker" binary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FirecrackerArguments {
    // main
    pub(crate) api_socket: FirecrackerApiSocket,
    id: Option<String>,
    config_path: Option<PathBuf>,
    // logging
    log_level: Option<FirecrackerLogLevel>,
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

pub enum FirecrackerConfigOverride {
    NoOverride,
    Disable,
    Enable(PathBuf),
}

impl FirecrackerArguments {
    pub fn new(api_socket: FirecrackerApiSocket) -> Self {
        Self {
            api_socket,
            id: None,
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

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn config_path(mut self, config_path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(config_path.into());
        self
    }

    pub fn log_level(mut self, log_level: FirecrackerLogLevel) -> Self {
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

    pub(crate) fn join(&self, config_override: FirecrackerConfigOverride) -> String {
        let mut args = HashMap::new();

        match &self.api_socket {
            FirecrackerApiSocket::Disabled => {
                args.insert("no-api".into(), ArgValue::None);
            }
            FirecrackerApiSocket::Enabled(socket_path) => {
                args.insert(
                    "api-sock".into(),
                    ArgValue::Some(socket_path.to_string_lossy().into_owned()),
                );
            }
        }

        if let Some(id) = &self.id {
            args.insert("id".into(), ArgValue::Some(id.to_owned()));
        }

        match config_override {
            FirecrackerConfigOverride::NoOverride => {
                if let Some(config_path) = &self.config_path {
                    args.insert(
                        "config-file".into(),
                        ArgValue::Some(config_path.to_string_lossy().into_owned()),
                    );
                }
            }
            FirecrackerConfigOverride::Disable => {}
            FirecrackerConfigOverride::Enable(path) => {
                args.insert(
                    "config-file".into(),
                    ArgValue::Some(path.to_string_lossy().into_owned()),
                );
            }
        }

        if let Some(log_level) = self.log_level {
            args.insert("level".into(), ArgValue::Some(log_level.into()));
        }

        if let Some(log_path) = &self.log_path {
            args.insert(
                "log-path".into(),
                ArgValue::Some(log_path.to_string_lossy().into_owned()),
            );
        }

        if self.show_log_origin {
            args.insert("show-log-origin".into(), ArgValue::None);
        }

        if let Some(module) = &self.log_module {
            args.insert("module".into(), ArgValue::Some(module.to_owned()));
        }

        if self.show_log_level {
            args.insert("show-level".into(), ArgValue::None);
        }

        if self.enable_boot_timer {
            args.insert("boot-timer".into(), ArgValue::None);
        }

        if let Some(max_payload) = self.api_max_payload_bytes {
            args.insert(
                "http-api-max-payload-size".into(),
                ArgValue::Some(max_payload.to_string()),
            );
        }

        if let Some(metadata_path) = &self.metadata_path {
            args.insert(
                "metadata".into(),
                ArgValue::Some(metadata_path.to_string_lossy().into_owned()),
            );
        }

        if let Some(metrics_path) = &self.metrics_path {
            args.insert(
                "metrics-path".into(),
                ArgValue::Some(metrics_path.to_string_lossy().into_owned()),
            );
        }

        if let Some(limit) = self.mmds_size_limit {
            args.insert("mmds-size-limit".into(), ArgValue::Some(limit.to_string()));
        }

        if self.disable_seccomp {
            args.insert("no-seccomp".into(), ArgValue::None);
        }

        if let Some(seccomp_path) = &self.seccomp_path {
            args.insert(
                "seccomp-filter".into(),
                ArgValue::Some(seccomp_path.to_string_lossy().into_owned()),
            );
        }

        join_args(args)
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

    pub fn resource_limits(
        mut self,
        resource_limits: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.resource_limits.extend(resource_limits);
        self
    }

    pub(crate) fn join(&self, firecracker_binary_path: &Path) -> String {
        let mut args = HashMap::new();
        args.insert(
            "exec-file".into(),
            ArgValue::Some(firecracker_binary_path.to_string_lossy().into_owned()),
        );
        args.insert("uid".into(), ArgValue::Some(self.uid.to_string()));
        args.insert("gid".into(), ArgValue::Some(self.gid.to_string()));
        args.insert("id".into(), ArgValue::Some(self.jail_id.to_string()));

        if !self.cgroup_values.is_empty() {
            args.insert(
                "cgroup".into(),
                ArgValue::Many(
                    self.cgroup_values
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect(),
                ),
            );
        }

        if let Some(cgroup_version) = self.cgroup_version {
            args.insert(
                "cgroup-version".into(),
                ArgValue::Some(match cgroup_version {
                    JailerCgroupVersion::V1 => "1".into(),
                    JailerCgroupVersion::V2 => "2".into(),
                }),
            );
        }

        if let Some(chroot_base_dir) = &self.chroot_base_dir {
            args.insert(
                "chroot-base-dir".into(),
                ArgValue::Some(chroot_base_dir.to_string_lossy().into_owned()),
            );
        }

        if self.daemonize {
            args.insert("daemonize".into(), ArgValue::None);
        }

        if let Some(network_namespace_path) = &self.network_namespace_path {
            args.insert(
                "netns".into(),
                ArgValue::Some(network_namespace_path.to_string_lossy().into_owned()),
            );
        }

        if self.exec_in_new_pid_ns {
            args.insert("new-pid-ns".into(), ArgValue::None);
        }

        if let Some(parent_cgroup) = &self.parent_cgroup {
            args.insert(
                "parent-cgroup".into(),
                ArgValue::Some(parent_cgroup.to_owned()),
            );
        }

        if !self.resource_limits.is_empty() {
            args.insert(
                "resource-limit".into(),
                ArgValue::Many(
                    self.resource_limits
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect(),
                ),
            );
        }

        join_args(args)
    }
}

/// A configuration of a Firecracker API Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FirecrackerApiSocket {
    /// The socket should be disabled via --no-api argument.
    Disabled,
    /// The socket should be enabled at the given path via --api-sock argument.
    Enabled(PathBuf),
}

/// A level of logging applied by Firecracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FirecrackerLogLevel {
    Off,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<FirecrackerLogLevel> for String {
    fn from(value: FirecrackerLogLevel) -> Self {
        match value {
            FirecrackerLogLevel::Off => "Off",
            FirecrackerLogLevel::Trace => "Trace",
            FirecrackerLogLevel::Debug => "Debug",
            FirecrackerLogLevel::Info => "Info",
            FirecrackerLogLevel::Warn => "Warn",
            FirecrackerLogLevel::Error => "Error",
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

enum ArgValue {
    None,
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
        FirecrackerApiSocket, FirecrackerArguments, FirecrackerConfigOverride, FirecrackerLogLevel,
        JailerArguments, JailerCgroupVersion,
    };

    #[test]
    fn firecracker_builds_correctly() {
        assert_fc(
            &["--no-api"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled),
        );
        assert_fc(
            &["--api-sock", "/tmp/socket.sock"],
            FirecrackerArguments::new(FirecrackerApiSocket::Enabled(PathBuf::from(
                "/tmp/socket.sock",
            ))),
        );
        assert_fc(
            &["--no-api", "--level Info"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled)
                .log_level(FirecrackerLogLevel::Info),
        );
        assert_fc(
            &["--no-api", "--log-path /tmp/t.fifo"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).log_path("/tmp/t.fifo"),
        );
        assert_fc(
            &["--no-api", "--show-log-origin"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).show_log_origin(),
        );
        assert_fc(
            &["--no-api", "--module mod"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).log_module("mod"),
        );
        assert_fc(
            &["--no-api", "--show-level"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).show_log_level(),
        );
        assert_fc(
            &["--no-api", "--boot-timer"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).enable_boot_timer(),
        );
        assert_fc(
            &["--no-api", "--http-api-max-payload-size 32"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).api_max_payload_bytes(32),
        );
        assert_fc(
            &["--no-api", "--metadata /tmp/metadata"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled)
                .metadata_path("/tmp/metadata"),
        );
        assert_fc(
            &["--no-api", "--metrics-path /tmp/m.path"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).metrics_path("/tmp/m.path"),
        );
        assert_fc(
            &["--no-api", "--mmds-size-limit 128"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).mmds_size_limit(128),
        );
        assert_fc(
            &["--no-api", "--no-seccomp"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled).disable_seccomp(),
        );
        assert_fc(
            &["--no-api", "--seccomp-filter /tmp/seccomp.filter"],
            FirecrackerArguments::new(FirecrackerApiSocket::Disabled)
                .seccomp_path("/tmp/seccomp.filter"),
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
            baseline
                .clone()
                .network_namespace_path("/var/run/netns/testing"),
        );
        assert_jailer(&["--new-pid-ns"], baseline.clone().exec_in_new_pid_ns());
        assert_jailer(
            &["--parent-cgroup cgroup"],
            baseline.clone().parent_cgroup("cgroup"),
        );
        assert_jailer(
            &["--resource-limit a=b", "--resource-limit c=d"],
            baseline
                .clone()
                .resource_limit("a", "b")
                .resource_limit("c", "d"),
        );
    }

    fn assert_fc(expectations: &[&str], args: FirecrackerArguments) {
        let joined_args = args.join(FirecrackerConfigOverride::NoOverride);
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
