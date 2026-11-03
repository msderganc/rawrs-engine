//! Bounded advisory record in the old main-header extension.

use crate::{crc32::crc32, producer};

pub(super) const RECORD_LEN: usize = 10 + producer::DATA.len();

pub(super) fn append(out: &mut Vec<u8>) {
    out.extend_from_slice(producer::NAME);
    out.extend_from_slice(&(producer::DATA.len() as u16).to_le_bytes());
    out.extend_from_slice(producer::DATA);
    out.extend_from_slice(&crc32(producer::DATA).to_le_bytes());
}

pub(super) fn replaceable_extra(extra: &[u8], comment: bool) -> bool {
    let record = if comment {
        let Ok(size) = super::read_u16(extra, 0) else {
            return false;
        };
        let Some(rest) = extra.get(usize::from(size) + 2..) else {
            return false;
        };
        rest
    } else {
        extra
    };
    if record.is_empty() {
        return true; // Original archives without a producer remain rewritable.
    }
    if record.len() < 10 || &record[..4] != producer::NAME {
        return false;
    }
    let size = usize::from(u16::from_le_bytes([record[4], record[5]]));
    if size as u64 > producer::MAX_DATA_LEN || record.len() != size + 10 {
        return false;
    }
    let data = &record[6..6 + size];
    let checksum = u32::from_le_bytes(record[6 + size..].try_into().unwrap());
    crc32(data) == checksum && producer::recognized(data)
}
