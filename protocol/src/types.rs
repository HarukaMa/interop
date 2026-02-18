use serde::{Deserialize, Serialize};

/// Frame type identifiers for the wire protocol.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    ExecRequest = 1,
    StdinData = 2,
    StdoutData = 3,
    StderrData = 4,
    ExitCode = 5,
    StdinClose = 6,
    PathQueryRequest = 7,
    PathQueryResponse = 8,
    Error = 9,
    TerminalResize = 10,
}

impl TryFrom<u8> for FrameType {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        match v {
            1 => Ok(Self::ExecRequest),
            2 => Ok(Self::StdinData),
            3 => Ok(Self::StdoutData),
            4 => Ok(Self::StderrData),
            5 => Ok(Self::ExitCode),
            6 => Ok(Self::StdinClose),
            7 => Ok(Self::PathQueryRequest),
            8 => Ok(Self::PathQueryResponse),
            9 => Ok(Self::Error),
            10 => Ok(Self::TerminalResize),
            other => Err(other),
        }
    }
}

/// A raw frame on the wire: 4-byte length + 1-byte type + payload.
#[derive(Debug, Clone)]
pub struct Frame {
    pub frame_type: FrameType,
    pub payload: Vec<u8>,
}

/// Request to execute a program on the Windows host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    /// Use ConPTY for interactive terminal support.
    #[serde(default)]
    pub interactive: bool,
    /// Terminal width (columns). Only used when interactive=true.
    #[serde(default = "default_term_cols")]
    pub term_width: u16,
    /// Terminal height (rows). Only used when interactive=true.
    #[serde(default = "default_term_rows")]
    pub term_height: u16,
}

fn default_term_cols() -> u16 {
    80
}
fn default_term_rows() -> u16 {
    24
}

/// Response to a PATH query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathQueryResponse {
    pub path_dirs: Vec<String>,
}
