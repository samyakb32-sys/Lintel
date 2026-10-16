// The wire between the speech client (speech.rs) and the speech server meteor-speak.exe
// (crates/voice-speak, which includes this very file by `#[path]`: no crate between the two, no cycle).
// VOICE §7.2. Owned by package voice-kokoro with both of its ends. Needs only std, serde and serde_json.
//
// stdin, one request per line:
//   {"id":7,"text":"Done.","voice":"af_heart","speed":1.0}
// stdout, one or more frames per request, in the order of the requests; a frame is a header line and
// then `samples` × f32 little-endian, mono, at `rate`:
//   {"id":7,"rate":24000,"samples":12000,"last":false}\n<48000 bytes>
// The last frame of a request has `last: true`. A frame with `error` has no samples and ends its request.
// Once the model is loaded the server writes one frame with id `READY` and no samples. Nothing else is
// ever written to stdout (what the engine says goes to stderr). The end of stdin ends the server.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

/// What Kokoro produces.
pub const RATE: u32 = 24_000;
/// The id of the frame that says "loaded"; requests start at 1.
pub const READY: u64 = 0;
/// A frame longer than a minute is a broken stream, not audio.
pub const MAX_SAMPLES: u32 = RATE * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub text: String,
    /// Kokoro's own name of the voice (`af_heart`).
    pub voice: String,
    /// 1.0 is the voice's own pace.
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub id: u64,
    pub rate: u32,
    pub samples: u32,
    pub last: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// Client: one request, flushed.
pub fn write_request(w: &mut impl Write, request: &Request) -> io::Result<()> {
    let mut line = serde_json::to_string(request).map_err(invalid)?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    w.flush()
}

/// Server: the next request; `None` at the end of stdin. Empty lines are skipped.
pub fn read_request(r: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            return serde_json::from_str(line.trim()).map(Some).map_err(invalid);
        }
    }
}

/// Server: one frame, flushed. `header.samples` is set from `pcm`.
pub fn write_frame(w: &mut impl Write, header: &Header, pcm: &[f32]) -> io::Result<()> {
    let header = Header { samples: pcm.len() as u32, ..header.clone() };
    let mut bytes = serde_json::to_vec(&header).map_err(invalid)?;
    bytes.push(b'\n');
    bytes.extend(pcm.iter().flat_map(|s| s.to_le_bytes()));
    w.write_all(&bytes)?;
    w.flush()
}

/// Client: the next frame. The end of the stream is `UnexpectedEof` (the server is gone).
pub fn read_frame(r: &mut impl BufRead) -> io::Result<(Header, Vec<f32>)> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let header: Header = serde_json::from_str(line.trim()).map_err(invalid)?;
    if header.samples > MAX_SAMPLES {
        return Err(invalid("frame too long"));
    }
    let mut bytes = vec![0u8; header.samples as usize * 4];
    r.read_exact(&mut bytes)?;
    let pcm = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Ok((header, pcm))
}
