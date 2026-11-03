# Later RAR 2.9-format Container Fixtures (rar420)

Cross-version fixtures from a later RAR 2.9-format writer, copied from the
spec repository's `fixtures/1.5-4.x/rar420/` set.

| Fixture | Purpose |
|---|---|
| `ext_time_rar420.rar` | Extended-time (`LHD_EXTTIME`) nibble groups for mtime/ctime/atime. |

The `rar420` header-encrypted fixture is stored in the sibling `encrypted/`
directory as `header_rar420_password.rar`, because the tests group all
password-protected fixtures together.
