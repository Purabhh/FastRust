# FastRust Streaming Body Architecture

**Status: DRAFT — Awaiting User Approval**
**Date: 2026-03-13**

---

## 1. Problem Statement

### 1.1 Problem A — Request Size Limit Checked After Full Buffering

`take_body_bytes()` collects ALL bytes via `incoming.collect().await` before comparing to `max_body_size`. A 500 MB upload against a 1 MB limit still buffers 500 MB first — trivial DoS vector.

### 1.2 Problem B — Compression Buffers Entire Response

`CompressionMiddleware` calls `BodyExt::collect()` on the full response before gzip encoding. Large file downloads sit entirely in heap before one compressed byte is written.

### 1.3 Problem C — Multipart Parser Materialises Full Body

`Multipart::from_request()` calls `ctx.take_body_bytes()` — a 50 MB file upload occupies 50 MB before the first field name is yielded.

### 1.4 Problem D — GET Handlers Pay for Body Collection They Never Use

`from_request::<B>()` calls `.collect().await` unconditionally for every request.

---

## 2. Proposed Types

### 2.1 BodyStream

```rust
// rust_squared/src/body.rs (new file)
pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, RsqError>> + Send>>;
```

### 2.2 LimitedBodyStream

Wraps any stream and aborts with 413 as soon as the running byte count exceeds `limit`:

```rust
pub struct LimitedBodyStream {
    inner: BodyStream,
    limit: usize,
    seen: usize,
}

impl Stream for LimitedBodyStream {
    type Item = Result<Bytes, RsqError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.seen += chunk.len();
                if self.seen > self.limit {
                    Poll::Ready(Some(Err(RsqError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!("payload exceeded {} bytes", self.limit),
                    ))))
                } else {
                    Poll::Ready(Some(Ok(chunk)))
                }
            }
            other => other,
        }
    }
}
```

### 2.3 Updated RsqRequestBody

```rust
pub enum RsqRequestBody {
    Streaming(Incoming),
    Limited(BodyStream),   // NEW: wraps Streaming with size enforcement
    Buffered(Bytes),
    Consumed,
}
```

### 2.4 New take_body_stream()

```rust
pub fn take_body_stream(&mut self) -> Result<BodyStream, RsqError> {
    // Returns the body as a stream, wrapping with LimitedBodyStream
    // Size limit enforced incrementally per chunk
}
```

### 2.5 GzipStreamBody — Streaming Compression

```rust
pub struct GzipStreamBody<B> {
    inner: B,
    encoder: GzEncoder<Vec<u8>>,
    done: bool,
}
// Implements http_body::Body — compresses one chunk at a time via Z_SYNC_FLUSH
```

### 2.6 StreamingMultipart

```rust
pub struct StreamingMultipart { /* ring-buffer boundary scanner */ }
pub struct StreamingPart {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: BodyStream,
}
impl StreamingMultipart {
    pub async fn next_part(&mut self) -> Result<Option<StreamingPart>, RsqError>;
}
```

---

## 3. What Breaks

| Item | Change | Fix |
|---|---|---|
| `RsqRequestBody` | New `Limited` variant | Add `Limited(_)` arm to any exhaustive match |
| Everything else | Unchanged signatures | No action needed |

**Recommendation:** Mark `RsqRequestBody` as `#[non_exhaustive]` before shipping Phase 1.

---

## 4. Implementation Phases

| Phase | Files | Problem | Risk |
|---|---|---|---|
| **1** — Incremental size limit | `body.rs` (new), `request.rs`, `lib.rs` | Problem A | Low |
| **2** — Streaming compression | `body.rs`, `middleware.rs` | Problem B | Medium |
| **3** — Streaming multipart | `multipart.rs`, `lib.rs` | Problem C | High — needs fuzz tests |
| **4** — Lazy GET collection | `request.rs` | Problem D | Very Low |

**Recommended order: 1, 4, 2, 3**

---

## 5. Open Questions — USER APPROVAL REQUIRED

### 5.1 Compression threshold strategy
- **Option A:** Peek-buffer up to `min_size` bytes before deciding — bounded memory, slight latency
- **Option B:** Always compress when client accepts gzip — simpler, negligible cost for small payloads ← recommended
- **Option C:** Status quo for small responses, streaming for large

### 5.2 BodyStream error type
- `RsqError` (consistent, recommended) vs `Box<dyn Error>` (flexible)

### 5.3 `#[non_exhaustive]` on RsqRequestBody
- Yes (forward-compatible) vs No (users can exhaustively match)

### 5.4 StreamingMultipart undrained parts
- Auto-skip undrained parts (ergonomic, complex) vs error on misuse (explicit, safer)

### 5.5 async-compression dependency
- Use `async-compression` crate for simpler Phase 2 vs implement `GzipStreamBody` from scratch with `flate2`

---

## 6. Key Implementation Notes

- **Production path already correct:** `handle_incoming_with_addr()` stores body as `Streaming(Incoming)`. Only the test-helper `handle()` path pre-collects. Phase 1 is primarily a `take_body_bytes()` change.
- **SSE is the reference pattern:** `SseBody<S>` in `sse.rs` is the exact structural template for `GzipStreamBody`.
- **No new dependencies required** for Phases 1, 3, 4. Phase 2 optionally benefits from `async-compression`.

*This document requires explicit user approval before any source files are modified.*
