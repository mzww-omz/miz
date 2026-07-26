# 0002: Preserve handle reservations after account purge

## Context

Phase 40 requires purged profiles and content to lose personal data and author linkage. The existing identity contract also makes handle claims permanent so deleted handles cannot be claimed by another account.

## Decision

Final purge marks every handle as non-current but retains its claim row. Normal profile and handle reads require an active account, so the retained value is not publicly resolvable. The user profile is anonymized, credentials and sessions are removed, and authored Posts are reassigned to the reserved deleted-account principal with their content removed.

## Alternatives

- Deleting handle rows would erase the identifier but allow impersonation through handle reuse.
- Storing only keyed handle hashes would require a new deployment secret and changing every handle-claim path; that is deferred until database-at-rest erasure is required.

## Consequences

Handles remain reserved and unavailable for registration after purge. The original handle remains in restricted database storage, so operators must treat the handles table as personal data. A future keyed-hash migration can remove the plaintext value without changing the public API.

## Compatibility and migration impact

No public API compatibility change is required. Existing handle uniqueness and retired-handle behavior continue to apply.
