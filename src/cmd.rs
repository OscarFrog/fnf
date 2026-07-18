use anyhow::Context;
use anyhow::Result;
use std::fmt::Display;
use std::process::Command;

/// Allows displaying a prepared command before executing it.
pub struct Cmd {
    program: String,
    args: Vec<String>,
}
impl Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.program, self.args.join(" "))
    }
}
impl Cmd {
    pub fn new(program: impl AsRef<str>) -> Self {
        Self {
            program: program.as_ref().to_string(),
            args: Vec::new(),
        }
    }
    pub fn arg(&mut self, arg: impl AsRef<str>) -> &mut Self {
        self.args.push(arg.as_ref().to_string());
        self
    }
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<str>>) -> &mut Self {
        for arg in args {
            self.arg(arg);
        }
        self
    }
    pub fn execute(self) -> Result<i32> {
        let mut command: Command = self.into();
        command.status()?.code().context("Failed to get exit code")
    }
}

impl Into<Command> for Cmd {
    fn into(self) -> Command {
        let mut command = Command::new(self.program);
        command.args(self.args);
        command
    }
}
