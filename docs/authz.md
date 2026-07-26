# Authorization matrix

Authorization is deny-by-default and checked against the authenticated user and target object on every request. Moderator and administrator APIs must use a separate router and audit every mutation.

| Action | Owner | Accepted follower | Third party | Moderator | Administrator |
|---|---:|---:|---:|---:|---:|
| Read public profile/post | Allow | Allow | Allow | Allow | Allow |
| Read followers-only post | Allow | Allow | Deny | Allow for moderation | Allow |
| Update/delete own profile/post | Allow | Deny | Deny | Deny | Deny |
| Moderate reported content | Deny | Deny | Deny | Allow | Allow |
| Manage users, roles, or system settings | Deny | Deny | Deny | Deny | Allow |

Email, birth date, credentials, session tokens, and consent records are private fields and are never exposed by public-resource authorization.
