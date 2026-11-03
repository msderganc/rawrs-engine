//! Recognition is deliberately stricter than ordinary unknown-subblock reading.

use super::{Archive, FileHeader, LONG_BLOCK, NEWSUB_HEAD};

pub(super) fn is_replaceable(file: &FileHeader, archive: &Archive) -> bool {
    if file.name != crate::producer::NAME
        || file.block.head_type != NEWSUB_HEAD
        || file.block.flags != LONG_BLOCK | 0x4000
        || file.block.head_size != 36
        || file.block.add_size != Some(file.pack_size)
        || !matches!(file.unp_ver, 15 | 20 | 29)
        || file.method != 0x30
        || file.host_os != 0
        || file.attr != 0
        || file.file_time != 0
        || file.salt.is_some()
        || file.unicode_name.is_some()
        || !file.ext_time.is_empty()
        || !file.file_comment.is_empty()
        || file.unp_size > crate::producer::MAX_DATA_LEN
        || file.pack_size != file.unp_size
        || file.packed_range.len() as u64 != file.unp_size
    {
        return false;
    }
    let mut data = Vec::new();
    file.write_to(archive, None, &mut data).is_ok() && crate::producer::recognized(&data)
}
