# Infrastructure

Deployment and operations configuration belongs here.

Run `miz-maintain` at least daily as a one-shot job. It purges expired report evidence and audit records, expires temporary restrictions, and processes account-deletion requests using `DATABASE_URL`; retry and concurrent execution are safe.

Create the first, dedicated administrator once by piping its password to `miz-operator-bootstrap` with `DATABASE_URL`, `OPERATOR_USERNAME`, and `OPERATOR_MFA_ENCRYPTION_KEY` set. Store the printed TOTP URI and recovery codes securely, then remove the output. The command refuses to run after any operator exists.
