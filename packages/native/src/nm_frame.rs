//! Chrome Native Messaging framing: a 32-bit length in native byte order, then UTF-8 JSON.
//! Chrome accepts at most 1 MB per message from the host, and sends at most 64 MB to it.

use std::io::{self, Read, Write};

pub const MAX_TO_CHROME: usize = 1024 * 1024;
pub const MAX_FROM_CHROME: usize = 64 * 1024 * 1024;

/// Reads one frame. `Ok(None)` at a clean end of input (Chrome closed the pipe).
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated length")),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_FROM_CHROME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one frame and flushes. Refuses bodies Chrome would reject (over 1 MB).
pub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_TO_CHROME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "message over 1 MB"));
    }
    w.write_all(&(body.len() as u32).to_ne_bytes())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_frames() {
        let mut buf = Vec::new();
        write_frame(&mut buf, br#"{"type":"hello"}"#).unwrap();
        write_frame(&mut buf, b"").unwrap();
        assert_eq!(&buf[..4], &16u32.to_ne_bytes());
        let mut r = Cursor::new(buf);
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), br#"{"type":"hello"}"#);
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"");
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn accepts_exactly_one_megabyte_and_refuses_more() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &vec![b'a'; MAX_TO_CHROME]).unwrap();
        assert_eq!(buf.len(), MAX_TO_CHROME + 4);
        let err = write_frame(&mut Vec::new(), &vec![b'a'; MAX_TO_CHROME + 1]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn reports_truncated_input() {
        let mut r = Cursor::new(vec![5u8, 0]);
        assert_eq!(read_frame(&mut r).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        let mut body = 10u32.to_ne_bytes().to_vec();
        body.extend_from_slice(b"abc");
        assert_eq!(read_frame(&mut Cursor::new(body)).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn refuses_an_absurd_length() {
        let body = ((MAX_FROM_CHROME + 1) as u32).to_ne_bytes().to_vec();
        assert_eq!(read_frame(&mut Cursor::new(body)).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
