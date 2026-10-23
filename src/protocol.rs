// Every TCP message is a 16-byte little-endian header plus `payload_len` payload bytes

//  Offset  Size  Field
//       0     4  magic: ASCII "AVDQ"
//       4     1  version: 1
//       5     1  message type
//       6     2  flags: 0 (reserved)
//       8     4  sequence/correlation id (u32 LE)
//      12     4  payload length in bytes (u32 LE)

pub const MAGIC: [u8; 4] = *b"AVDQ";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 16;
pub const MAX_PAYLOAD_LEN: usize = 1536;

// Fixed payload for the first HELLO demonstration. This is NOT the final device capabilities/configuration schema
pub const HELLO_PAYLOAD: &[u8] = b"link-firmware:avdq-v1";

use crate::sample_block::{
    SampleBlock, CHANNEL_COUNT, SCANS_PER_BLOCK,
};

// Data payload (all integers little-endian) appended after 16-byte AVDQ header:
//  0: u64 first_scan_index
//  8: u32 sample_rate_hz (2000)
// 12: u16 scan_count (100)
// 14: u8  channel_count (2)
// 15: u8  sample_encoding (1 = signed i32 ADC counts, little endian)
// 16: u32 cumulative dropped 100-scan blocks since START
// 20: u32 source_flags (bit 0 = synthesized data)
// 24: 100 x [i32 channel_0, i32 channel_1] = 800 bytes
pub const DATA_METADATA_LEN: usize = 24;
pub const PACKED_SAMPLE_BYTES: usize = 3;
pub const DATA_ENCODING_I24_LE: u8 = 1;
pub const DATA_PAYLOAD_LEN: usize = DATA_METADATA_LEN + SCANS_PER_BLOCK * CHANNEL_COUNT * PACKED_SAMPLE_BYTES;
pub const DATA_SOURCE_SYNTHETIC: u32 = 1;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Hello = 0x01,
    Configure = 0x02,
    ConfigureReply = 0x03,
    Start = 0x04,
    Data = 0x05,
    Status = 0x06,
    Stop = 0x07,
    Stopped = 0x08,
    Error = 0x09,
    // Temporary diagnostic commands
    Ping = 0x10,
    Pong = 0x11,
}

impl MessageType {
    pub const fn from_wire(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Hello),
            0x02 => Some(Self::Configure),
            0x03 => Some(Self::ConfigureReply),
            0x04 => Some(Self::Start),
            0x05 => Some(Self::Data),
            0x06 => Some(Self::Status),
            0x07 => Some(Self::Stop),
            0x08 => Some(Self::Stopped),
            0x09 => Some(Self::Error),
            0x10 => Some(Self::Ping),
            0x11 => Some(Self::Pong),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    BadMagic,
    UnsupportedVersion,
    UnexpectedFlags,
    OversizedPayload,
    OutputTooSmall,
    InvalidScanCount,
    InvalidSampleValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub message_type: u8,
    pub sequence: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    pub fn new(
        message_type: MessageType,
        sequence: u32,
        payload_len: usize,
    ) -> Result<Self, ProtocolError> {
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::OversizedPayload);
        }
        Ok(Self {
            message_type: message_type as u8,
            sequence,
            payload_len: payload_len as u32,
        })
    }

    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.message_type;
        // out[6..8] reserved flags, currently zero
        out[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        out[12..16].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    pub fn decode(input: &[u8; HEADER_LEN]) -> Result<Self, ProtocolError> {
        if input[..4] != MAGIC[..] {
            return Err(ProtocolError::BadMagic);
        }
        if input[4] != VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if input[6] != 0 || input[7] != 0 {
            return Err(ProtocolError::UnexpectedFlags);
        }

        let payload_len = u32::from_le_bytes([
            input[12], input[13], input[14], input[15],
        ]);
        if payload_len > MAX_PAYLOAD_LEN as u32 {
            return Err(ProtocolError::OversizedPayload);
        }

        Ok(Self {
            message_type: input[5],
            sequence: u32::from_le_bytes([
                input[8], input[9], input[10], input[11],
            ]),
            payload_len,
        })
    }
}

// Construct a complete, fixed-length DATA payload. This is pure Rust and
// does not require a heap, SPI, an ADC, or a TCP socket
pub fn encode_data_payload(
    block: &SampleBlock,
    dropped_blocks: u32,
    output: &mut [u8; DATA_PAYLOAD_LEN],
) -> Result<usize, ProtocolError> {
    let count = block.scan_count as usize;
    if count == 0 || count > SCANS_PER_BLOCK {
        return Err(ProtocolError::InvalidScanCount);
    }
    output[..8].copy_from_slice(&block.first_scan_index.to_le_bytes());
    output[8..12].copy_from_slice(&block.sample_rate_hz.to_le_bytes());
    output[12..14].copy_from_slice(&block.scan_count.to_le_bytes());
    output[14] = CHANNEL_COUNT as u8;
    output[15] = DATA_ENCODING_I24_LE;
    output[16..20].copy_from_slice(&dropped_blocks.to_le_bytes());
    output[20..24].copy_from_slice(&DATA_SOURCE_SYNTHETIC.to_le_bytes());

    let mut offset = DATA_METADATA_LEN;
    for scan in block.samples[..count].iter() {
        for &value in scan.iter() {
            if !(-8_388_608..=8_388_607).contains(&value) {
                return Err(ProtocolError::InvalidSampleValue);
            }
            let bytes = value.to_le_bytes();
            output[offset..offset + PACKED_SAMPLE_BYTES]
                .copy_from_slice(&bytes[..PACKED_SAMPLE_BYTES]);
            offset += PACKED_SAMPLE_BYTES;
        }
    }
    Ok(offset)
}

// Encode an entire AVDQ frame into caller-supplied memory (no heap)
pub fn encode_frame(
    message_type: MessageType,
    sequence: u32,
    payload: &[u8],
    output: &mut [u8],
) -> Result<usize, ProtocolError> {
    let header = FrameHeader::new(message_type, sequence, payload.len())?.encode();
    let used = HEADER_LEN + payload.len();
    if output.len() < used {
        return Err(ProtocolError::OutputTooSmall);
    }
    output[..HEADER_LEN].copy_from_slice(&header);
    output[HEADER_LEN..used].copy_from_slice(payload);
    Ok(used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample_block::{synthetic_block, synthetic_scan};

    fn decode_i24_le_for_test(bytes: &[u8]) -> i32 {
        let raw = u32::from(bytes[0])
            | (u32::from(bytes[1]) << 8)
            | (u32::from(bytes[2]) << 16);
        ((raw << 8) as i32) >> 8
    }

    #[test]
    fn signed_24_bit_boundaries_round_trip() {
        let values = [-8_388_608i32, -1, 0, 1, 8_388_607];
        let expected = [
            [0x00, 0x00, 0x80],
            [0xff, 0xff, 0xff],
            [0x00, 0x00, 0x00],
            [0x01, 0x00, 0x00],
            [0xff, 0xff, 0x7f],
        ];
        for (value, want) in values.into_iter().zip(expected) {
            let bytes = value.to_le_bytes();
            assert_eq!(bytes[..3], want);
            assert_eq!(decode_i24_le_for_test(&want), value);
        }
    }

    #[test]
    fn variable_data_payload_layout() {
        assert!(DATA_PAYLOAD_LEN <= MAX_PAYLOAD_LEN);
        assert_eq!(DATA_PAYLOAD_LEN, 1224); // 200 * 2 * 3 + 24
        for (scans, rate) in [(100u16, 2_000u32), (200u16, 64_000u32)] {
            let block = synthetic_block(300, scans, rate);
            let mut payload = [0u8; DATA_PAYLOAD_LEN];
            let used = encode_data_payload(&block, 7, &mut payload).unwrap();
            assert_eq!(used, DATA_METADATA_LEN + scans as usize * 2 * 3);
            assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 300);
            assert_eq!(u32::from_le_bytes(payload[8..12].try_into().unwrap()), rate);
            assert_eq!(u16::from_le_bytes(payload[12..14].try_into().unwrap()), scans);
            assert_eq!(payload[14], 2);
            assert_eq!(payload[15], DATA_ENCODING_I24_LE);
            assert_eq!(u32::from_le_bytes(payload[16..20].try_into().unwrap()), 7);
            assert_eq!(u32::from_le_bytes(payload[20..24].try_into().unwrap()), 1);
            let ch0 = decode_i24_le_for_test(&payload[24..27]);
            let ch1 = decode_i24_le_for_test(&payload[27..30]);
            assert_eq!([ch0, ch1], synthetic_scan(300, rate));
            let last = DATA_METADATA_LEN + (scans as usize - 1) * 6;
            assert_eq!(decode_i24_le_for_test(&payload[last..last+3]), synthetic_scan(300 + scans as u64 - 1, rate)[0]);
        }
    }

    #[test]
    fn header_wire_bytes_are_little_endian() {
        let header = FrameHeader::new(MessageType::Ping, 0x1234_5678, 3).unwrap();
        assert_eq!(
            header.encode(),
            [
                0x41, 0x56, 0x44, 0x51, // AVDQ
                0x01, 0x10,             // version, ping type
                0x00, 0x00,             // flags
                0x78, 0x56, 0x34, 0x12, // sequence
                0x03, 0x00, 0x00, 0x00, // payload len
            ],
        );
    }

    #[test]
    fn frame_encodes_and_decodes() {
        let mut out = [0u8; 64];
        let len = encode_frame(MessageType::Pong, 42, b"\x00\xffhello", &mut out).unwrap();
        assert_eq!(len, HEADER_LEN + 7);
        let header: &[u8; HEADER_LEN] = out[..HEADER_LEN].try_into().unwrap();
        let parsed = FrameHeader::decode(header).unwrap();
        assert_eq!(parsed.message_type, MessageType::Pong as u8);
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.payload_len, 7);
        assert_eq!(&out[HEADER_LEN..len], b"\x00\xffhello");
    }

    #[test]
    fn malformed_headers_are_rejected() {
        let mut h = FrameHeader::new(MessageType::Hello, 0, 0).unwrap().encode();
        h[0] = b'X';
        assert_eq!(FrameHeader::decode(&h), Err(ProtocolError::BadMagic));
        h[0] = b'A';
        h[4] = 2;
        assert_eq!(FrameHeader::decode(&h), Err(ProtocolError::UnsupportedVersion));
        h[4] = VERSION;
        h[6] = 1;
        assert_eq!(FrameHeader::decode(&h), Err(ProtocolError::UnexpectedFlags));
        h[6] = 0;
        h[12..16].copy_from_slice(&((MAX_PAYLOAD_LEN as u32) + 1).to_le_bytes());
        assert_eq!(FrameHeader::decode(&h), Err(ProtocolError::OversizedPayload));
    }

    #[test]
    fn no_buffer_overflows() {
        let mut too_small = [0u8; HEADER_LEN];
        assert_eq!(
            encode_frame(MessageType::Ping, 1, b"a", &mut too_small),
            Err(ProtocolError::OutputTooSmall)
        );
        let payload = [0u8; MAX_PAYLOAD_LEN + 1];
        let mut large_out = [0u8; MAX_PAYLOAD_LEN + HEADER_LEN + 1];
        assert_eq!(
            encode_frame(MessageType::Ping, 1, &payload, &mut large_out),
            Err(ProtocolError::OversizedPayload)
        );
    }
}
