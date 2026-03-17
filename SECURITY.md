# Security Policy

## Reporting a Vulnerability

To report a security vulnerability in FastRust, please **do not** open a public GitHub issue.
Instead, email the maintainers privately or open a [GitHub private security advisory](https://github.com/Purabhh/FastRust/security/advisories/new).

Include as much detail as possible:
- A description of the vulnerability and its impact
- Steps to reproduce or a proof-of-concept
- Affected versions
- Suggested remediation (if any)

You can expect an initial acknowledgement within 72 hours and a resolution timeline within 14 days for critical issues.

---

## Security-Relevant Configuration Notes

### CORS — use `with_origins()` in production

`CorsMiddleware::permissive()` sets `Access-Control-Allow-Origin: *`, which allows any origin to
make cross-origin requests to your API. **Never use `permissive()` in production.**

Use the allowlist constructor instead:

```rust
CorsMiddleware::with_origins(vec!["https://app.example.com", "https://admin.example.com"])
```

### X-Forwarded-For trust — deploy behind a trusted reverse proxy

`RateLimitMiddleware` uses `X-Forwarded-For` as a fallback key when no `peer_addr` is available.
This header can be forged by clients unless your deployment ensures only a trusted reverse proxy
(nginx, Cloudflare, AWS ALB, etc.) can set it.

**Always deploy FastRust behind a reverse proxy in production** so the peer address is set
correctly and `X-Forwarded-For` is injected by the proxy, not the client.

---

## Known Open Issues

### FINDING 6 — CSRF token lifecycle (MEDIUM)

**Status:** Partially mitigated (token comparison is now constant-time; see C-3 fix).

**Remaining issue:** The CSRF token is generated fresh on every safe (GET/HEAD/OPTIONS) response and
set as a `SameSite=Strict; Secure` cookie, but there is no server-side token store or per-session
binding. A CSRF token received in one response is valid for any subsequent state-changing request
until it is rotated. Applications that require strict per-request CSRF protection should implement
a server-side token store (e.g. keyed by session ID) and rotate tokens on each use.

### FINDING 8 — Internal error disclosure (LOW)

**Status:** Open.

**Description:** `RsqError::internal()` propagates the raw internal error message string to the
HTTP response body (status 500). In production this may leak implementation details, stack traces,
or sensitive path/data information to the client.

**Recommended fix:** Replace `RsqError::internal()` response bodies with a generic message
(`"internal server error"`) and log the detailed error server-side via `tracing::error!`.
