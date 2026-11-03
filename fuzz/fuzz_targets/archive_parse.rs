#![no_main]

use libfuzzer_sys::fuzz_target;
use rars::ArchiveReader;

fuzz_target!(|data: &[u8]| {
    let _ = ArchiveReader::read(data);
});
