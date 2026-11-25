//! Fixed-width wire format for the first USB copy fault in a boot.
#![forbid(unsafe_code)]

pub(super) const MAGIC: [u8; 4] = *b"WUF1";
pub(super) const BYTES: usize = 64;
const END: usize = BYTES - 4;

pub(super) fn crc32(bytes: &[u8]) -> u32 {
    // CRC-32/ISO-HDLC also used by the independent host decoder.
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

pub(super) fn encode(text: &str) -> Option<[u8; BYTES]> {
    let len = text.len();
    if text.is_empty() || len > END - 5 || !text.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
    {
        return None;
    }
    let mut record = [0; BYTES];
    record[..4].copy_from_slice(&MAGIC);
    record[4] = u8::try_from(len).ok()?;
    record[5..END][..len].copy_from_slice(text.as_bytes());
    let checksum = crc32(&record[..END]).to_le_bytes();
    record[END..].copy_from_slice(&checksum);
    Some(record)
}

pub(super) fn message(record: &[u8; BYTES]) -> Option<&str> {
    if record[..4] != MAGIC || crc32(&record[..END]).to_le_bytes() != record[END..] {
        return None;
    }
    let bytes = record[5..END].get(..usize::from(record[4]))?;
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        return None;
    }
    core::str::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_host_and_all_single_bit_record_errors_are_rejected() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let text = "USB0 EP1 copy guard 0x3";
        let record = encode(text).expect("short ASCII fault");
        assert_eq!(message(&record), Some(text));
        for byte in 0..record.len() {
            for bit in 0..8 {
                let mut bad = record;
                bad[byte] ^= 1 << bit;
                assert_eq!(message(&bad), None);
            }
        }
        assert_eq!(encode(""), None);
        assert_eq!(encode("USB0\n"), None);
        let mut bad = record;
        bad[4] = 255;
        let crc = crc32(&bad[..END]).to_le_bytes();
        bad[END..].copy_from_slice(&crc);
        assert_eq!(message(&bad), None);
    }
}
