use bytes::{Buf, BufMut, BytesMut};

use crate::types::{Frame, FrameType};

/// Encode a frame into the buffer.
pub fn encode_frame(frame: &Frame, buf: &mut BytesMut) {
    let payload_len = frame.payload.len();
    // Length field covers type byte + payload
    buf.put_u32(payload_len as u32 + 1);
    buf.put_u8(frame.frame_type as u8);
    buf.put_slice(&frame.payload);
}

/// Try to decode a frame from the buffer. Returns None if not enough data yet.
pub fn decode_frame(buf: &mut BytesMut) -> Result<Option<Frame>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }

    let length = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if length == 0 {
        return Err(DecodeError::ZeroLength);
    }

    if buf.len() < 4 + length {
        return Ok(None);
    }

    buf.advance(4); // consume length
    let type_byte = buf[0];
    buf.advance(1); // consume type

    let frame_type = FrameType::try_from(type_byte).map_err(DecodeError::UnknownType)?;
    let payload = buf.split_to(length - 1).to_vec();

    Ok(Some(Frame {
        frame_type,
        payload,
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("zero-length frame")]
    ZeroLength,
    #[error("unknown frame type: {0}")]
    UnknownType(u8),
}

// Convenience constructors

pub fn exec_request_frame(req: &crate::types::ExecRequest) -> Frame {
    Frame {
        frame_type: FrameType::ExecRequest,
        payload: serde_json::to_vec(req).unwrap(),
    }
}

pub fn stdin_data_frame(data: &[u8]) -> Frame {
    Frame {
        frame_type: FrameType::StdinData,
        payload: data.to_vec(),
    }
}

pub fn stdout_data_frame(data: &[u8]) -> Frame {
    Frame {
        frame_type: FrameType::StdoutData,
        payload: data.to_vec(),
    }
}

pub fn stderr_data_frame(data: &[u8]) -> Frame {
    Frame {
        frame_type: FrameType::StderrData,
        payload: data.to_vec(),
    }
}

pub fn exit_code_frame(code: i32) -> Frame {
    Frame {
        frame_type: FrameType::ExitCode,
        payload: code.to_be_bytes().to_vec(),
    }
}

pub fn stdin_close_frame() -> Frame {
    Frame {
        frame_type: FrameType::StdinClose,
        payload: vec![],
    }
}

pub fn path_query_request_frame() -> Frame {
    Frame {
        frame_type: FrameType::PathQueryRequest,
        payload: vec![],
    }
}

pub fn path_query_response_frame(resp: &crate::types::PathQueryResponse) -> Frame {
    Frame {
        frame_type: FrameType::PathQueryResponse,
        payload: serde_json::to_vec(resp).unwrap(),
    }
}

pub fn error_frame(msg: &str) -> Frame {
    Frame {
        frame_type: FrameType::Error,
        payload: msg.as_bytes().to_vec(),
    }
}

/// Terminal resize frame: 2 bytes width + 2 bytes height (big-endian).
pub fn terminal_resize_frame(width: u16, height: u16) -> Frame {
    let mut payload = Vec::with_capacity(4);
    payload.extend_from_slice(&width.to_be_bytes());
    payload.extend_from_slice(&height.to_be_bytes());
    Frame {
        frame_type: FrameType::TerminalResize,
        payload,
    }
}

/// Parse width and height from a TerminalResize frame payload.
pub fn parse_terminal_resize(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() < 4 {
        return None;
    }
    let width = u16::from_be_bytes([payload[0], payload[1]]);
    let height = u16::from_be_bytes([payload[2], payload[3]]);
    Some((width, height))
}
