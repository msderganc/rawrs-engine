//! Shared advisory producer payload; each container supplies its own framing.

pub(crate) const NAME: &[u8] = b"RARS";
pub(crate) const DATA: &[u8] =
    concat!("rars/", env!("CARGO_PKG_VERSION"), "\nmarker-schema=1\n").as_bytes();
pub(crate) const MAX_DATA_LEN: u64 = 128;

pub(crate) fn recognized(data: &[u8]) -> bool {
    data.strip_prefix(b"rars/")
        .and_then(|data| data.strip_suffix(b"\nmarker-schema=1\n"))
        .is_some_and(|version| {
            !version.is_empty()
                && version
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(byte))
        })
}
