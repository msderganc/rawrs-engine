# JavaScript RAR Library Fixture

Small third-party archive-comment fixture copied from the external
JavaScript RAR library corpus.

| Fixture | Purpose |
|---|---|
| `with_comment.rar` | RAR 3.x `CMT` NEWSUB archive comment with UTF-16LE text. It exercises a larger Unpack29 comment payload than the local `rar300` ASCII comment fixture. |

The original corpus also contains password-protected examples; those are either
represented in the `encrypted/` directory or kept out of positive tests when no
clean reference oracle exists.
