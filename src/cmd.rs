use anyhow::Context;
use anyhow::Result;
use std::fmt::Display;
use std::process::Command;

/// Allows displaying a prepared command before executing it.
pub struct Cmd {
    program: String,
    args: Vec<String>,
    envs: Vec<(String, String)>,
}
impl Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.envs.is_empty() {
            let envs = self
                .envs
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>();
            write!(f, "{} ", envs.join(" "))?;
        }
        write!(f, "{}", self.program)?;
        if !self.args.is_empty() {
            write!(f, " {}", self.args.join(" "))?;
        }
        Ok(())
    }
}
impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
        }
    }
    pub fn arg(&mut self, arg: impl Into<String>) -> &mut Self {
        self.args.push(arg.into());
        self
    }
    pub fn args(&mut self, args: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        for arg in args {
            self.arg(arg);
        }
        self
    }
    pub fn env(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.envs.push((key.into(), value.into()));
        self
    }
    pub fn execute(self) -> Result<i32> {
        Command::from(self)
            .status()?
            .code()
            .context("Failed to get exit code")
    }
}

impl From<Cmd> for Command {
    fn from(cmd: Cmd) -> Self {
        let mut command = Command::new(cmd.program);
        command.args(cmd.args);
        for (key, value) in cmd.envs {
            command.env(key, value);
        }
        command
    }
}
