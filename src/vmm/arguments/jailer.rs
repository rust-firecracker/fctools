use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::vmm::id::VmmId;

/// Arguments that can be passed into the "jailer" binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JailerArguments {
    pub(crate) jail_id: VmmId,
    cgroup_values: HashMap<String, String>,
    cgroup_version: Option<JailerCgroupVersion>,
    pub(crate) chroot_base_dir: Option<PathBuf>,
    pub(crate) daemonize: bool,
    network_namespace_path: Option<PathBuf>,
    pub(crate) exec_in_new_pid_ns: bool,
    parent_cgroup: Option<String>,
    max_file_size_limit: Option<u64>,
    max_fd_limit: Option<u64>,
}

impl JailerArguments {
    /// Create new [JailerArguments] tied to the provided [VmmId] as the jail ID.
    pub fn new(jail_id: VmmId) -> Self {
        Self {
            jail_id,
            cgroup_values: HashMap::new(),
            cgroup_version: None,
            chroot_base_dir: None,
            daemonize: false,
            network_namespace_path: None,
            exec_in_new_pid_ns: false,
            parent_cgroup: None,
            max_file_size_limit: None,
            max_fd_limit: None,
        }
    }

    /// Add a cgroup key-value pair to the [JailerArguments].
    pub fn cgroup<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.cgroup_values.insert(key.into(), value.into());
        self
    }

    /// Add an iterator of cgroup key-value pairs to the [JailerArguments].
    pub fn cgroups<I: IntoIterator<Item = (String, String)>>(mut self, cgroups: I) -> Self {
        self.cgroup_values.extend(cgroups);
        self
    }

    /// Specify the jailer's [JailerCgroupVersion].
    pub fn cgroup_version(mut self, cgroup_version: JailerCgroupVersion) -> Self {
        self.cgroup_version = Some(cgroup_version);
        self
    }

    /// Specify the path to the base chroot directory for the jailer.
    pub fn chroot_base_dir<P: Into<PathBuf>>(mut self, chroot_base_dir: P) -> Self {
        self.chroot_base_dir = Some(chroot_base_dir.into());
        self
    }

    /// Enable jailer daemonization, which is disabled by default.
    pub fn daemonize(mut self) -> Self {
        self.daemonize = true;
        self
    }

    /// Specify the path (usually located within either /var/run/netns or /run/netns when using iproute2 or fcnet
    /// for setting up microVM networking) to the network namespace for the jailer to enter.
    pub fn network_namespace_path<P: Into<PathBuf>>(mut self, network_namespace_path: P) -> Self {
        self.network_namespace_path = Some(network_namespace_path.into());
        self
    }

    /// Enable execution within a newly created PID namespace, which is disabled by default.
    pub fn exec_in_new_pid_ns(mut self) -> Self {
        self.exec_in_new_pid_ns = true;
        self
    }

    /// Specify a parent cgroup for the jailer.
    pub fn parent_cgroup<C: Into<String>>(mut self, parent_cgroup: C) -> Self {
        self.parent_cgroup = Some(parent_cgroup.into());
        self
    }

    /// Specify the limit on the maximum size of files created by the Firecracker process spawned by the jailer,
    /// in bytes.
    pub fn max_file_size_limit(mut self, max_file_size_limit: u64) -> Self {
        self.max_file_size_limit = Some(max_file_size_limit);
        self
    }

    /// Specify the limit on the highest file descriptor allocated by the Firecracker process spawned by the jailer.
    pub fn max_fd_limit(mut self, max_fd_limit: u64) -> Self {
        self.max_fd_limit = Some(max_fd_limit);
        self
    }

    /// Join the [JailerArguments] into a [Vec] of process arguments, using the given jailer target UID and GID as
    /// well as a [Path] to the "firecracker" binary. The order in which the argument [String]s are inserted into
    /// the resulting [Vec] is not stable!
    pub fn join(&self, uid: u32, gid: u32, firecracker_binary_path: &Path) -> Vec<String> {
        let mut args = Vec::with_capacity(8);
        args.push("--exec-file".to_string());
        args.push(firecracker_binary_path.to_string_lossy().into_owned());
        args.push("--uid".to_string());
        args.push(uid.to_string());
        args.push("--gid".to_string());
        args.push(gid.to_string());
        args.push("--id".to_string());
        args.push(self.jail_id.as_ref().to_owned());

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

        if let Some(max_file_size_limit) = self.max_file_size_limit {
            args.push("--resource-limit".to_string());
            args.push(format!("fsize={max_file_size_limit}"));
        }

        if let Some(max_fd_limit) = self.max_fd_limit {
            args.push("--resource-limit".to_string());
            args.push(format!("no-file={max_fd_limit}"));
        }

        args
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::vmm::id::VmmId;

    use super::{JailerArguments, JailerCgroupVersion};

    fn new() -> JailerArguments {
        JailerArguments::new(VmmId::new("jail-id").unwrap())
    }

    #[test]
    fn uid_gid_jail_id_are_pushed() {
        check(new(), ["--uid", "1", "--gid", "--id", "jail-id"]);
    }

    #[test]
    fn cgroup_values_can_be_set() {
        check(new().cgroup("key", "value"), ["--cgroup", "key=value"]);
    }

    #[test]
    fn cgroup_version_can_be_set() {
        for (cgroup_version, matcher) in [(JailerCgroupVersion::V1, "1"), (JailerCgroupVersion::V2, "2")] {
            check(new().cgroup_version(cgroup_version), ["--cgroup-version", matcher]);
        }
    }

    #[test]
    fn chroot_base_dir_can_be_set() {
        check(
            new().chroot_base_dir("/tmp/chroot"),
            ["--chroot-base-dir", "/tmp/chroot"],
        );
    }

    #[test]
    fn daemonize_can_be_enabled() {
        check(new().daemonize(), ["--daemonize"]);
    }

    #[test]
    fn netns_can_be_set() {
        check(
            new().network_namespace_path("/var/run/netns"),
            ["--netns", "/var/run/netns"],
        );
    }

    #[test]
    fn exec_in_new_pid_ns_can_be_enabled() {
        check(new().exec_in_new_pid_ns(), ["--new-pid-ns"]);
    }

    #[test]
    fn parent_cgroup_can_be_set() {
        check(
            new().parent_cgroup("parent_cgroup"),
            ["--parent-cgroup", "parent_cgroup"],
        );
    }

    #[test]
    fn max_file_size_limit_can_be_set() {
        check(new().max_file_size_limit(250), ["--resource-limit", "fsize=250"]);
    }

    #[test]
    fn max_fd_limit_can_be_set() {
        check(new().max_fd_limit(100), ["--resource-limit", "no-file=100"]);
    }

    fn check<const AMOUNT: usize>(args: JailerArguments, matchers: [&str; AMOUNT]) {
        let joined_args = args.join(1, 1, &PathBuf::from("/tmp/firecracker"));
        assert!(joined_args.contains(&String::from("--exec-file")));
        assert!(joined_args.contains(&String::from("/tmp/firecracker")));

        for matcher in matchers {
            assert!(joined_args.contains(&matcher.to_string()));
        }
    }
}
