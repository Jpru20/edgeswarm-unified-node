use flate2::{read::MultiGzDecoder, write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const HEADER: usize = 48;
const MAGIC: &[u8; 4] = b"SWCT";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Task = 1,
    Retrieval = 2,
    Result = 3,
    Stream = 4,
    Conversation = 5,
}

impl Kind {
    fn limit(self) -> usize {
        match self {
            Self::Task => 2 << 20,
            Self::Retrieval => 8 << 20,
            Self::Result | Self::Conversation => 4 << 20,
            Self::Stream => 256 << 10,
        }
    }
}

pub enum Transfer {
    Legacy(Vec<u8>),
    Compact(Vec<u8>),
}

pub fn encode(kind: Kind, raw: &[u8]) -> Result<Vec<u8>, &'static str> {
    if raw.len() > kind.limit() {
        return Err("RAW_LIMIT_EXCEEDED");
    }

    let mut body = raw.to_vec();
    let mut codec = 0_u8;

    if raw.len() >= 512 {
        let mut gz = GzEncoder::new(Vec::new(), Compression::new(3));
        gz.write_all(raw).map_err(|_| "GZIP_ENCODE_FAILED")?;
        let candidate = gz.finish().map_err(|_| "GZIP_ENCODE_FAILED")?;

        if raw.len().saturating_sub(candidate.len()) >= 128
            && candidate.len() * 100 <= raw.len() * 92
        {
            body = candidate;
            codec = 1;
        }
    }

    if body.len() > kind.limit() + (64 << 10) {
        return Err("ENCODED_LIMIT_EXCEEDED");
    }

    let mut frame = Vec::with_capacity(HEADER + body.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&[VERSION, kind as u8, codec, 0]);
    frame.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&Sha256::digest(raw));
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode(kind: Kind, frame: &[u8]) -> Result<Vec<u8>, &'static str> {
    if frame.len() < HEADER {
        return Err("FRAME_TOO_SHORT");
    }
    if &frame[..4] != MAGIC {
        return Err("BAD_MAGIC");
    }
    if frame[4] != VERSION {
        return Err("BAD_VERSION");
    }
    if frame[5] != kind as u8 {
        return Err("WRONG_KIND");
    }
    if frame[7] != 0 {
        return Err("BAD_FLAGS");
    }

    let raw_len = u32::from_be_bytes(frame[8..12].try_into().unwrap()) as usize;
    let encoded_len = u32::from_be_bytes(frame[12..16].try_into().unwrap()) as usize;

    if raw_len > kind.limit() {
        return Err("RAW_LIMIT_EXCEEDED");
    }
    if encoded_len > kind.limit() + (64 << 10) {
        return Err("ENCODED_LIMIT_EXCEEDED");
    }
    if frame.len() != HEADER + encoded_len {
        return Err("BAD_LENGTH");
    }

    let output = match frame[6] {
        0 => {
            if raw_len != encoded_len {
                return Err("IDENTITY_LENGTH_MISMATCH");
            }
            frame[HEADER..].to_vec()
        }
        1 => {
            let decoder = MultiGzDecoder::new(&frame[HEADER..]);
            let mut output = Vec::new();
            decoder
                .take((kind.limit() + 1) as u64)
                .read_to_end(&mut output)
                .map_err(|_| "GZIP_DECODE_FAILED")?;
            if output.len() > kind.limit() {
                return Err("DECOMPRESSION_LIMIT_EXCEEDED");
            }
            output
        }
        _ => return Err("UNSUPPORTED_CODEC"),
    };

    if output.len() != raw_len {
        return Err("RAW_LENGTH_MISMATCH");
    }

    let actual = Sha256::digest(&output);
    let mut differences = 0_u8;
    for (a, b) in actual.iter().zip(&frame[16..48]) {
        differences |= *a ^ *b;
    }
    if differences != 0 {
        return Err("PAYLOAD_HASH_MISMATCH");
    }

    Ok(output)
}

pub fn choose(kind: Kind, raw: &[u8]) -> Result<Transfer, &'static str> {
    if raw.len() > kind.limit() {
        return Err("RAW_LIMIT_EXCEEDED");
    }
    if raw.len() < 1024 || (kind == Kind::Stream && raw.len() < 4096) {
        return Ok(Transfer::Legacy(raw.to_vec()));
    }

    let frame = encode(kind, raw)?;
    let saved = raw.len().saturating_sub(frame.len());

    if saved >= 128 && saved * 100 >= raw.len() * 8 {
        Ok(Transfer::Compact(frame))
    } else {
        Ok(Transfer::Legacy(raw.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_roundtrip_all_kinds() {
        let raw = b" {\"source\":\"a\", \"quote\":\"Revenue $248M\"} \n";
        for kind in [
            Kind::Task,
            Kind::Retrieval,
            Kind::Result,
            Kind::Stream,
            Kind::Conversation,
        ] {
            let decoded = decode(kind, &encode(kind, raw).unwrap()).unwrap();
            assert_eq!(decoded.as_slice(), raw);
        }
    }

    #[test]
    fn compress_retrieval_and_memory() {
        for kind in [Kind::Retrieval, Kind::Conversation] {
            let raw = "citation a: Revenue $248M\n".repeat(1000).into_bytes();
            let frame = match choose(kind, &raw).unwrap() {
                Transfer::Compact(f) => f,
                Transfer::Legacy(_) => panic!("expected savings"),
            };
            assert_eq!(frame[6], 1);
            assert_eq!(decode(kind, &frame).unwrap(), raw);
        }
    }

    #[test]
    fn small_stream_stays_legacy() {
        assert!(matches!(
            choose(Kind::Stream, b"hello").unwrap(),
            Transfer::Legacy(_)
        ));
    }

    #[test]
    fn corruption_and_wrong_kind_fail_closed() {
        let mut frame = encode(Kind::Result, b"signed result").unwrap();
        assert!(decode(Kind::Task, &frame).is_err());
        frame[16] ^= 1;
        assert_eq!(
            decode(Kind::Result, &frame).unwrap_err(),
            "PAYLOAD_HASH_MISMATCH"
        );
        frame[4] = 2;
        assert_eq!(decode(Kind::Result, &frame).unwrap_err(), "BAD_VERSION");
    }

    #[test]
    fn stream_size_cap_enforced() {
        assert_eq!(
            encode(Kind::Stream, &vec![0; (256 << 10) + 1]).unwrap_err(),
            "RAW_LIMIT_EXCEEDED"
        );
    }
}

#[cfg(test)]
mod swct_end_to_end_canary_v1 {
    use super::*;
    use base64::Engine;
    use sha2::Digest;

    #[test]
    fn node_js_wire_fixtures_decode() {
        let task = "U1dDVAEBAAAAAAA9AAAAPdctp4+5ELWnbe62ft5HXZI0ptx5Da4h9QY9FGo0TWfxICB7InRhc2tJZCI6NzcsICJwcm9tcHQiOiLDhMOWIMOlIOS9oOWlvSDwn4yNIiwibW9kZWwiOiI3YiJ9Cg==";
        let result = "U1dDVAEDAQAAAABCAAAAU7A/FFUpxJRc4sW3pyd0fFTOmoaz2x6ZoNv+uUYiDUr/H4sIAAAAAAAAA1NQqFZKLClJzS0o8UxRslJKNFTSUUrMjM8vLSkoLVGyUvLJzEtVMIzJA9NGSjpKafn5JalFSlZKj+ZMUapV4AIAHWKwiEIAAAA=";
        let b64 = base64::engine::general_purpose::STANDARD;
        let original = decode(Kind::Task, &b64.decode(task).unwrap()).unwrap();
        let reply = decode(Kind::Result, &b64.decode(result).unwrap()).unwrap();
        assert!(String::from_utf8(original).unwrap().contains("你好 🌍"));
        assert!(String::from_utf8(reply).unwrap().contains("✔"));
    }

    #[test]
    fn selected_context_binary_roundtrip() {
        let history = "Customer requires private devices.";
        let memory = "Enterprise pricing uses subscriptions.";
        let hash = |s: &str| -> String {
            sha2::Sha256::digest(s.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let packet = serde_json::json!({
            "version": "swarm-selected-context-v1",
            "taskId": "test-101",
            "planId": "plan-101",
            "excerpts": [
                {"kind":"conversation_user","id":"c1","text":history,"sha256":hash(history)},
                {"kind":"persistent_memory","id":"m1","text":memory,"sha256":hash(memory)}
            ]
        });
        let context =
            crate::core::swarm_selected_context_v1::assemble(&packet.to_string(), "test-101")
                .unwrap();
        let frame = encode(Kind::Conversation, context.as_bytes()).unwrap();
        assert_eq!(
            decode(Kind::Conversation, &frame).unwrap(),
            context.as_bytes()
        );
        assert!(context.find("c1").unwrap() < context.find("m1").unwrap());
        assert!(crate::core::swarm_selected_context_v1::assemble(
            &packet.to_string(),
            "wrong-task"
        )
        .is_err());
    }
}
