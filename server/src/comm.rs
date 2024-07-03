use base64::Engine;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use serde::{Deserialize, Serialize};


type RawChunk = [u8; CHUNK_SIZE / 8];

#[derive(Debug, Deserialize, Serialize)]
pub struct ClientState<'a> {
    b64: &'a str,
}

impl ClientState<'_> {
    fn to_raw_chunk(&self) -> Result<RawChunk, base64::DecodeSliceError> {
        let mut chunk = [0; CHUNK_SIZE / 8];

        BASE64_STANDARD_NO_PAD.decode_slice(self.b64, &mut chunk)?;

        Ok(chunk)
    }
}
