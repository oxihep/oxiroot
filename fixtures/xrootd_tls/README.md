# Test certificates for the XRootD TLS tests

**Test-only. Never trust these anywhere else.** The CA's private key was thrown
away after signing; `server.key` is public on purpose.

- `ca.pem` — the "oxiroot test CA" (P-256). Pass this directory as the CA
  directory (`XrootdOptions::ca_dir`, or `X509_CERT_DIR`) to trust it.
- `server.pem`, `server.key` — a server certificate for `localhost` and
  `127.0.0.1`, signed by the CA, valid for 100 years.

Used by the mock XRootD server in `crates/oxiroot/tests/it/remote_xrootd.rs`
and by `scripts/xrootd_tls_local.sh`, which runs a real `xrootd` with TLS and
token (`ztn`) authentication.
