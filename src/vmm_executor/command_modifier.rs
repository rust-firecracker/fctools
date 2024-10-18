use std::{fmt::Debug, path::PathBuf};

/// A command modifier is a simple transformation that can be applied to a &mut Vec<String> of arguments
/// and a &mut PathBuf binary path. This allows customizing executor behavior beyond the scope of what the
/// FirecrackerArguments and JailerArguments take into consideration, such as prepending, appending or
/// replacing parts of the command string. Multiple command modifiers can also be chained together.
pub trait CommandModifier: Debug + Send + Sync {
    /// Apply the modification to the given args and binary path.
    fn apply(&self, binary_path: &mut PathBuf, args: &mut Vec<String>);
}

/// A command modifier that wraps the "firecracker"/"jailer" invocation behind iproute2's "netns exec" command
/// in order to put the spawned process in a certain network namespace via the iproute2 utility.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetnsCommandModifier {
    netns_name: String,
    iproute2_path: PathBuf,
}

impl NetnsCommandModifier {
    pub fn new(netns_name: impl Into<String>) -> Self {
        Self {
            netns_name: netns_name.into(),
            iproute2_path: PathBuf::from("/usr/sbin/ip"),
        }
    }

    pub fn iproute2_path(mut self, iproute2_path: impl Into<PathBuf>) -> Self {
        self.iproute2_path = iproute2_path.into();
        self
    }
}

impl CommandModifier for NetnsCommandModifier {
    fn apply(&self, binary_path: &mut PathBuf, args: &mut Vec<String>) {
        let original_binary_path = binary_path.to_string_lossy().into_owned();
        *binary_path = self.iproute2_path.clone();
        args.insert(0, "netns".to_string());
        args.insert(1, "exec".to_string());
        args.insert(2, self.netns_name.clone());
        args.insert(3, original_binary_path);
    }
}

pub(crate) fn apply_command_modifier_chain(
    binary_path: &mut PathBuf,
    args: &mut Vec<String>,
    modifiers: &Vec<Box<dyn CommandModifier>>,
) {
    for modifier in modifiers {
        modifier.apply(binary_path, args);
    }
}
