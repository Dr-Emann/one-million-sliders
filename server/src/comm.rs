use base64::Engine;
use base64::prelude::BASE64_STANDARD_NO_PAD;
use serde::{Deserialize, Serialize};
use crate::shared_bitmap;

#[derive(Debug, Deserialize, Serialize)]
pub struct ClientState<'a> {
    b64: &'a str,
}

impl ClientState<'_> {
    fn to_raw_chunk(&self) -> Result<shared_bitmap::Chunk, base64::DecodeSliceError> {
        let mut chunk = shared_bitmap::Chunk::new();

        BASE64_STANDARD_NO_PAD.decode_slice(self.b64, &mut chunk.0)?;

        Ok(chunk)
    }
}
