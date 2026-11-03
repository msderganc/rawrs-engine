# rars-generated RAR 1.5 Writer Oracles

Static copies of `rars`-generated RAR 1.5 writer fixtures from the spec repo.
They are validated with an external RAR 2.9-format reader and used by
`rar15_40_fixtures.rs` to pin emitted writer bytes and decoded payloads against
public-reader behavior. The spec-repo originals live under
`fixtures/1.5-4.x/rars-generated/`, where `scripts/verify-fixtures.py` checks
their SHA-256 table.

Password for encrypted fixtures: `pass`.

`comments.rar` was regenerated for 0.7. Its file comment used to be a bare size
and text, which is the RAR 1.3 layout and wrong from RAR 1.5 on; it is now the
comment block other RAR writers emit. The spec-repo copy and its SHA-256 entry
need the same replacement.
