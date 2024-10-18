use std::{fmt::Debug, path::PathBuf};

/// An argument modifier is a simple transformation that can be applied to a &mut Vec<String> of arguments.
/// This allows customizing executor behavior beyond the scope of what the
/// FirecrackerArguments and JailerArguments take into consideration, such as prepending, appending or
/// replacing parts of the command string. Multiple command modifiers can also be chained together.
pub trait ArgumentModifier: Debug + Send + Sync {
    /// Perform the modification of the given command passed by mutable reference.
    fn modify_args(&self, args: &mut Vec<String>);
}

/// An argument modifier that prepends "ip netns exec NETNS " to the actual command, thus utilizing iproute2
/// in order to move the process into a given network namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetnsArgumentModifier {
    netns_name: String,
    iproute2_path: PathBuf,
}

impl NetnsArgumentModifier {
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

impl ArgumentModifier for NetnsArgumentModifier {
    fn modify_args(&self, args: &mut Vec<String>) {
        args.insert(0, self.iproute2_path.to_string_lossy().into_owned());
        args.insert(1, "netns".to_string());
        args.insert(2, "exec".to_string());
    }
}

/// A command modifier that appends an arbitrary string to the end of the original command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppendCommandModifier {
    appended_args: Vec<String>,
}

impl AppendCommandModifier {
    pub fn new() -> Self {
        Self {
            appended_args: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            appended_args: Vec::with_capacity(capacity),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.appended_args.push(arg.into());
        self
    }
}

impl ArgumentModifier for AppendCommandModifier {
    fn modify_args(&self, args: &mut Vec<String>) {
        args.extend_from_slice(&self.appended_args);
    }
}

/// An argument modifier that replaces the entire argument vector with an arbitrary one. Not recommended for use
/// unless you are completely certain the rewrite will lead to the correct command!
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteArgumentModifier {
    rewrite_args: Vec<String>,
}

impl RewriteArgumentModifier {
    pub fn new() -> Self {
        Self {
            rewrite_args: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            rewrite_args: Vec::with_capacity(capacity),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.rewrite_args.push(arg.into());
        self
    }
}

impl ArgumentModifier for RewriteArgumentModifier {
    fn modify_args(&self, args: &mut Vec<String>) {
        args.clear();
        args.extend_from_slice(&self.rewrite_args);
    }
}

pub(crate) fn apply_argument_modifier_chain(args: &mut Vec<String>, modifiers: &Vec<Box<dyn ArgumentModifier>>) {
    for modifier in modifiers {
        modifier.modify_args(args);
    }
}
