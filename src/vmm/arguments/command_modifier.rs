use std::{fmt::Debug, path::PathBuf};

/// A [CommandModifier] is a simple transformation that can be applied to a [Vec<String>] of arguments
/// and a [PathBuf] binary path. This allows customizing argument behavior beyond the scope of what the
/// [VmmArguments] and [JailerArguments] take into consideration, such as prepending, appending or
/// replacing parts of the command [String]. Multiple [CommandModifier] should be chained together and
/// executed in the exact order they were configured.
pub trait CommandModifier: Debug + Send + Sync + 'static {
    /// Apply the modification to the given arguments and binary path.
    fn apply(&self, binary_path: &mut PathBuf, arguments: &mut Vec<String>);
}

/// A [CommandModifier] that wraps the "firecracker"/"jailer" invocation behind iproute2's "netns exec" command
/// in order to put the spawned process in a certain network namespace via the iproute2 utility.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetnsCommandModifier {
    netns_name: String,
    iproute2_path: PathBuf,
}

impl NetnsCommandModifier {
    /// Create a new [NetnsCommandModifier] from a given name of a network namespace.
    pub fn new<N: Into<String>>(netns_name: N) -> Self {
        Self {
            netns_name: netns_name.into(),
            iproute2_path: PathBuf::from("/usr/sbin/ip"),
        }
    }

    /// Override the path to iproute2 used by this [NetnsCommandModifier]. The default one is "/usr/sbin/ip".
    pub fn iproute2_path<P: Into<PathBuf>>(mut self, iproute2_path: P) -> Self {
        self.iproute2_path = iproute2_path.into();
        self
    }
}

impl CommandModifier for NetnsCommandModifier {
    fn apply(&self, binary_path: &mut PathBuf, arguments: &mut Vec<String>) {
        let original_binary_path = binary_path.to_string_lossy().into_owned();
        *binary_path = self.iproute2_path.clone();
        arguments.insert(0, "netns".to_string());
        arguments.insert(1, "exec".to_string());
        arguments.insert(2, self.netns_name.clone());
        arguments.insert(3, original_binary_path);
    }
}

#[cfg(test)]
#[test]
fn netns_command_modifier_performs_changes() {
    let command_modifier = NetnsCommandModifier::new("my_netns").iproute2_path("/sbin/ip");
    let mut binary_path = PathBuf::from("/opt/binary");
    let mut arguments = vec!["run".to_string(), "my".to_string(), "stuff".to_string()];
    command_modifier.apply(&mut binary_path, &mut arguments);
    assert_eq!(binary_path.to_str().unwrap(), "/sbin/ip");
    assert_eq!(
        arguments,
        vec!["netns", "exec", "my_netns", "/opt/binary", "run", "my", "stuff"]
    )
}
