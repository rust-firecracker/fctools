use std::{collections::HashMap, fmt::Debug, path::PathBuf};

pub trait CommandModifier: Debug + Sync {
    fn modify_command(&self, command: &mut String);
}

#[derive(Debug, Default)]
pub struct NoCommandModifier {}

impl CommandModifier for NoCommandModifier {
    fn modify_command(&self, _command: &mut String) {}
}

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
    fn modify_command(&self, command: &mut String) {
        *command = format!(
            "{} netns exec {} {command}",
            self.iproute2_path.to_string_lossy(),
            self.netns_name
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppendCommandModifier {
    appended_command: String,
}

impl AppendCommandModifier {
    pub fn new(appended_command: impl Into<String>) -> Self {
        Self {
            appended_command: appended_command.into(),
        }
    }
}

impl CommandModifier for AppendCommandModifier {
    fn modify_command(&self, command: &mut String) {
        command.push_str(&self.appended_command);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteCommandModifier {
    new_command: String,
}

impl RewriteCommandModifier {
    pub fn new(new_command: impl Into<String>) -> Self {
        Self {
            new_command: new_command.into(),
        }
    }
}

impl CommandModifier for RewriteCommandModifier {
    fn modify_command(&self, command: &mut String) {
        *command = self.new_command.clone();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceCommandModifier {
    replacements: HashMap<String, String>,
}

impl ReplaceCommandModifier {
    pub fn new() -> Self {
        Self {
            replacements: HashMap::new(),
        }
    }

    pub fn replace(mut self, original: impl Into<String>, new: impl Into<String>) -> Self {
        self.replacements.insert(original.into(), new.into());
        self
    }
}

impl CommandModifier for ReplaceCommandModifier {
    fn modify_command(&self, command: &mut String) {
        for (original, new) in &self.replacements {
            *command = command.replace(original, new);
        }
    }
}

pub(crate) fn apply_command_modifier_chain(command: &mut String, modifiers: &Vec<Box<dyn CommandModifier>>) {
    for modifier in modifiers {
        modifier.modify_command(command);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppendCommandModifier, CommandModifier, NetnsCommandModifier, NoCommandModifier, ReplaceCommandModifier,
        RewriteCommandModifier,
    };

    #[test]
    fn no_command_modifier_does_nothing() {
        assert_modifier(NoCommandModifier::default(), "something", "something");
    }

    #[test]
    fn netns_command_modifier_uses_default_iproute2() {
        assert_modifier(
            NetnsCommandModifier::new("test"),
            "command",
            "/usr/sbin/ip netns exec test command",
        );
    }

    #[test]
    fn netns_command_modifier_uses_custom_iproute2() {
        assert_modifier(
            NetnsCommandModifier::new("test").iproute2_path("/custom/path"),
            "command",
            "/custom/path netns exec test command",
        );
    }

    #[test]
    fn append_command_modifier_performs_action() {
        assert_modifier(AppendCommandModifier::new("appended"), "command", "commandappended");
    }

    #[test]
    fn rewrite_command_modifier_performs_action() {
        assert_modifier(RewriteCommandModifier::new("rewritten"), "original", "rewritten");
    }

    #[test]
    fn replace_command_modifier_performs_action() {
        assert_modifier(
            ReplaceCommandModifier::new().replace("a", "b").replace("c", "d"),
            "ac",
            "bd",
        );
    }

    fn assert_modifier(modifier: impl CommandModifier, from: &str, to: &str) {
        let mut command = from.to_owned();
        modifier.modify_command(&mut command);
        assert_eq!(command, to);
    }
}
