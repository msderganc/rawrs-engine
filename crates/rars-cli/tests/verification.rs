use rars::rar50::{ArchiveEntry, Rar50Writer, WriterOptions};
use rars::{ArchiveVersion, EntrySource, FeatureSet, FilterKind, FilterPolicy, FilterSpec};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/scratch.rs"]
mod scratch;

fn archive(path: &Path, data: Vec<u8>, policy: FilterPolicy) {
    let options = WriterOptions::new(ArchiveVersion::Rar50, FeatureSet::store_only())
        .with_compression_level(3)
        .with_dictionary_size(128 * 1024);
    let bytes = Rar50Writer::new(options)
        .entry(ArchiveEntry::new(
            b"payload".to_vec(),
            EntrySource::from_bytes(data),
        ))
        .filter_policy(policy)
        .finish()
        .unwrap();
    fs::write(path, bytes).unwrap();
}

fn test(paths: &[PathBuf], options: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rars"))
        .arg("test")
        .args(options)
        .args(paths)
        .output()
        .unwrap()
}

fn success(result: &Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn default_test_avoids_a_second_output_buffer_and_preserves_decode_routing() {
    let root = scratch::case("verification-workspace");
    let path = root.join("archive.rar");
    archive(
        &path,
        b"abc"
            .iter()
            .copied()
            .cycle()
            .take(4 * 1024 * 1024)
            .collect(),
        FilterPolicy::None,
    );
    let result = test(
        std::slice::from_ref(&path),
        &["--max-reader-workspace-bytes", "7m"],
    );
    success(&result);
    assert_eq!(result.stdout, b"OK payload\n");
    let streamed = test(
        std::slice::from_ref(&path),
        &[
            "--max-reader-workspace-bytes",
            "1m",
            "--rar50-buffered-decode-limit",
            "0",
        ],
    );
    success(&streamed);
    assert_eq!(streamed.stdout, b"OK payload\n");
    let default_small = test(
        std::slice::from_ref(&path),
        &["--max-reader-workspace-bytes", "1m"],
    );
    assert!(!default_small.status.success());
    assert!(default_small.stdout.is_empty());
    let result = test(
        &[path],
        &[
            "--max-reader-workspace-bytes",
            "1m",
            "--rar50-buffered-decode-limit",
            "8m",
        ],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("workspace"));
}

#[test]
fn default_test_does_not_retain_stored_member_output() {
    let root = scratch::case("verification-stored-workspace");
    let path = root.join("archive.rar");
    let mut builder = rars::Builder::new(ArchiveVersion::Rar50).store(true);
    for name in [b"first", b"other"] {
        builder
            .add_bytes(name.to_vec(), vec![0x53; 4 * 1024 * 1024], None, None)
            .unwrap();
    }
    fs::write(&path, builder.to_bytes().unwrap()).unwrap();
    let result = test(&[path], &["--max-reader-workspace-bytes", "256k"]);
    success(&result);
    assert_eq!(result.stdout, b"OK first\nOK other\n");
}

#[test]
fn filtered_test_preserves_limits_and_publishes_success_only_after_verification() {
    let root = scratch::case("verification-filtered");
    let path = root.join("archive.rar");
    let mut data = b"abc"
        .iter()
        .copied()
        .cycle()
        .take(300 * 1024)
        .collect::<Vec<_>>();
    data.extend_from_slice(&b"\xe8\0\0\0\0filtered bytes\n".repeat(2048));
    let count = data.len().to_string();
    let policy = FilterPolicy::Explicit(FilterSpec::range(FilterKind::E8, 300 * 1024..data.len()));
    archive(&path, data, policy);
    let result = test(
        std::slice::from_ref(&path),
        &["--max-total-output-bytes", &count],
    );
    success(&result);
    assert_eq!(result.stdout, b"OK payload\n");
    let result = test(
        std::slice::from_ref(&path),
        &["--rar50-buffered-decode-limit", "0"],
    );
    success(&result);
    assert_eq!(result.stdout, b"OK payload\n");
    let limit = (count.parse::<usize>().unwrap() - 1).to_string();
    let result = test(&[path], &["--max-total-output-bytes", &limit]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn encrypted_and_solid_split_verification_prints_each_logical_member_once() {
    for (prefix, count, padded, password) in [
        ("encrypted_multivol", 3, false, Some("password")),
        ("solid_multivol", 6, true, None),
    ] {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rars/tests/fixtures/rar50");
        let paths = (1..=count)
            .map(|index| {
                let number = if padded {
                    format!("{index:02}")
                } else {
                    index.to_string()
                };
                fixture.join(format!("{prefix}.part{number}.rar"))
            })
            .collect::<Vec<_>>();
        let options = password.map_or_else(Vec::new, |password| vec!["--password", password]);
        let result = test(&paths, &options);
        success(&result);
        let archives = paths
            .iter()
            .map(|path| {
                rars::ArchiveReader::read_path_with_options(
                    path,
                    rars::ArchiveReadOptions::with_optional_password(password.map(str::as_bytes)),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let mut names = Vec::new();
        rars::test_volumes_with_options_and_callback(
            &archives,
            rars::ArchiveReadOptions::with_optional_password(password.map(str::as_bytes)),
            |meta| {
                names.push(meta.name.clone());
                Ok(())
            },
        )
        .unwrap();
        let expected = names
            .iter()
            .map(|name| format!("OK {}\n", String::from_utf8_lossy(name)))
            .collect::<String>();
        assert_eq!(result.stdout, expected.as_bytes());
        if password.is_some() {
            let failed = test(&paths, &["--password", "wrong"]);
            assert!(!failed.status.success());
            assert!(failed.stdout.is_empty());
        }
    }
}
