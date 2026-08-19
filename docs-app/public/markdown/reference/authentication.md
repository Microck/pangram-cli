# Authentication reference

Precedence is `PANGRAM_API_KEY`, then the protected stored credential. Use
`pangram auth set`, `pangram auth status`, and `pangram auth logout` to manage
the stored value. Persistent credentials require owner-only permissions or a
protected owner-only Windows ACL. The CLI fails closed when it cannot prove
those permissions.
