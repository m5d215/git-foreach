use crate::runner::Stream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Echo of the executed command (`$ <command>`).
    Command,
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct OutputLine {
    pub kind: LineKind,
    pub text: String,
}

/// Output for a single repo. Rebuilt on every Run (previous output is discarded).
#[derive(Debug, Default, Clone)]
pub struct RepoOutput {
    pub lines: Vec<OutputLine>,
    pub code: Option<i32>,
    pub finished: bool,
}

impl RepoOutput {
    pub fn push_command(&mut self, command: &str) {
        self.lines.push(OutputLine {
            kind: LineKind::Command,
            text: format!("$ {command}"),
        });
    }

    pub fn push_stream(&mut self, stream: Stream, text: String) {
        let kind = match stream {
            Stream::Stdout => LineKind::Stdout,
            Stream::Stderr => LineKind::Stderr,
        };
        self.lines.push(OutputLine { kind, text });
    }
}
