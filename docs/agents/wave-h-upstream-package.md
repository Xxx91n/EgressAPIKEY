# Wave-H Upstream Delivery Package — SSE per-event flush (Resinat/Resin)

Slug: `r12-wave-h-grill` | Ledger: D-002 (sequenced upstream-first → dormant patch-fork), D-003 (execution boundary: full local production, zero external action)
Patch base: upstream `master @ 9b8ef8e` (2026-08-01, `feat(webui): add lease account search`)
Patch file (raw, `git apply`-able, tracked): `docs/agents/wave-h-sse-flush.patch`
(working copy also at `.scratch/r12-wave-h-grill/wave-h-sse-flush.patch`)
Commit message (prescribed, D-003.1): `forward: flush SSE responses per event`

> Agent produced everything below locally. No fork repo, issue, or PR was
> created — external actions under the user's GitHub identity are user-side
> (D-003.4). The resin/ clone working tree carries the same changes.

## 1. Patch summary

`internal/proxy/forward.go` (~L294): the forward HTTP path streamed every
upstream response body through a bare `io.Copy(w, resp.Body)`. `net/http`'s
response writer buffers ~4KB, so SSE events (`Content-Type: text/event-stream`)
arrived at the client coalesced into buffer-fill windows instead of per-event.
The patch routes the copy through `copyForwardResponseBody`, which detects the
SSE media type (`mime.ParseMediaType`, case-insensitive, parameter-tolerant)
and, when the `ResponseWriter` supports `http.Flusher`, copies through
`sseFlushWriter` (Write → Flush per read). Non-SSE responses keep the identical
`io.Copy` path byte-for-byte; SSE behind a non-Flusher writer degrades to the
old path safely.

Forward-side diff shown whitespace-normalized for readability (tabs -> spaces);
the byte-exact, apply-ready patch including the test half is
`docs/agents/wave-h-sse-flush.patch`:

```diff
diff --git a/internal/proxy/forward.go b/internal/proxy/forward.go
index cb815a1..7cfe77c 100644
--- a/internal/proxy/forward.go
+++ b/internal/proxy/forward.go
@@ -5,6 +5,7 @@ import (
  "encoding/base64"
  "errors"
  "io"
+ "mime"
  "net/http"
  "net/http/httptrace"
  "strings"
@@ -291,7 +292,7 @@ func (p *ForwardProxy) handleHTTP(w http.ResponseWriter, r *http.Request) {
  // Copy end-to-end response headers and body.
  lifecycle.addIngressBytes(copyEndToEndHeaders(w.Header(), resp.Header))
  w.WriteHeader(resp.StatusCode)
- copiedBytes, copyErr := io.Copy(w, resp.Body)
+ copiedBytes, copyErr := copyForwardResponseBody(w, resp)
  lifecycle.addIngressBytes(copiedBytes)
  if copyErr != nil {
   if shouldRecordForwardCopyFailure(r, copyErr) {
@@ -407,6 +408,39 @@ func (p *ForwardProxy) handleCONNECT(w http.ResponseWriter, r *http.Request) {
  prepare.session.recordResult(relay.netOK)
 }
 
+// isEventStreamContentType reports whether a Content-Type header value is a
+// server-sent-events stream ("text/event-stream", optionally with parameters).
+func isEventStreamContentType(contentType string) bool {
+ mediaType, _, err := mime.ParseMediaType(contentType)
+ return err == nil && mediaType == "text/event-stream"
+}
+
+// sseFlushWriter flushes after every Write so each read chunk is delivered to
+// the client immediately instead of waiting for the server's write buffer to
+// fill.
+type sseFlushWriter struct {
+ w http.ResponseWriter
+ f http.Flusher
+}
+
+func (fw sseFlushWriter) Write(p []byte) (int, error) {
+ n, err := fw.w.Write(p)
+ fw.f.Flush()
+ return n, err
+}
+
+// copyForwardResponseBody streams an upstream response body to the client.
+// SSE responses flush per read so events arrive per-event; all other bodies
+// keep the plain io.Copy path byte-for-byte.
+func copyForwardResponseBody(w http.ResponseWriter, resp *http.Response) (int64, error) {
+ if isEventStreamContentType(resp.Header.Get("Content-Type")) {
+  if flusher, ok := w.(http.Flusher); ok {
+   return io.Copy(sseFlushWriter{w: w, f: flusher}, resp.Body)
+  }
+ }
+ return io.Copy(w, resp.Body)
+}
+
 // shouldRecordForwardCopyFailure decides whether an HTTP response body copy
 // error should be treated as an upstream/node failure.
 func shouldRecordForwardCopyFailure(r *http.Request, copyErr error) bool {
```

The test half (`internal/proxy/forward_http_copy_test.go`, +241 lines) is in
the patch file; it adds `TestIsEventStreamContentType` (5 subtests),
`TestCopyForwardResponseBody_SSEFlushesAfterEveryWrite` (write→flush
interleave assertion), `TestCopyForwardResponseBody_SSEEventArrivesBeforeStreamEnds`
(per-event arrival through a real `httptest` server with a channel-gated
upstream body — the first event is read while the second is still withheld),
`TestCopyForwardResponseBody_NonSSEUnchanged` (zero flushes + byte-identical
body), and `TestCopyForwardResponseBody_SSEWithoutFlusherStillCopies`
(fallback path).

## 2. Test evidence (host, Go 1.26.2 / windows-amd64)

- `go build ./internal/proxy/` — PASS. (`go build ./...` on this host fails at `webui/embed.go` — it requires the generated, gitignored `webui/dist`; the touched package builds clean.)
- `go test ./internal/proxy/ -run 'TestCopyForwardResponseBody|TestIsEventStreamContentType|TestShouldRecordForwardCopyFailure' -v` — PASS (all 10 subtests/tests).
- `go test ./internal/proxy/` — 1 failure: `TestPumpPreparedTunnelReader_ClientReadResetAfterIngressDoesNotFail` (`wsarecv: An existing connection was forcibly closed`). **Pre-existing, reproduced on unmodified master @ 9b8ef8e** (verified via `git stash` + re-run): a Windows TCP-reset semantics mismatch in a CONNECT-tunnel test, unrelated to this patch. Worth one line in the probe issue if maintainers ask.

## 3. PR draft

Title: `forward: flush SSE responses per event`

Body:

```markdown
## Symptom

When proxying a `text/event-stream` response through the forward HTTP path,
SSE events reach the client coalesced into ~4KB windows instead of one write
per event. Small or slow event streams sit in the response write buffer until
it fills or the body closes, so clients observe stalled/batched delivery.

## Root cause

`internal/proxy/forward.go` `handleHTTP` copies every upstream response body
with a bare `io.Copy(w, resp.Body)` (~L294). `net/http`'s response writer
buffers up to 4096 bytes before writing to the socket; nothing ever calls
`Flush` mid-body.

## Fix

Route the copy through `copyForwardResponseBody`: on
`Content-Type: text/event-stream` (parsed via `mime.ParseMediaType`, so
parameters like `; charset=utf-8` are tolerated) and when the writer supports
`http.Flusher`, copy through a `sseFlushWriter` that calls `Flush()` after
every `Write`. Non-SSE responses keep the previous `io.Copy` path
byte-for-byte; an SSE response behind a non-Flusher writer falls back to the
same plain copy.

## Tests

`internal/proxy/forward_http_copy_test.go`:

- `TestCopyForwardResponseBody_SSEEventArrivesBeforeStreamEnds` — end-to-end
  per-event arrival through `httptest.NewServer`: event 1 is read while a
  channel gate withholds event 2 (would time out under the old path).
- `TestCopyForwardResponseBody_SSEFlushesAfterEveryWrite` — write→flush
  interleaving assertion.
- `TestCopyForwardResponseBody_NonSSEUnchanged` — zero flushes, byte-identical
  body (invariance of the old path).
- `TestCopyForwardResponseBody_SSEWithoutFlusherStillCopies` — fallback.
- `TestIsEventStreamContentType` — detection edge cases.

`go build ./internal/proxy/` and `go test ./internal/proxy/` run clean for
the touched code (one unrelated pre-existing failure in
`TestPumpPreparedTunnelReader_ClientReadResetAfterIngressDoesNotFail` on this
Windows host — reproduces on unmodified master).

---

This change was drafted with AI assistance; the submitter has verified it and
can defend the approach.
```

## 4. Probe-issue draft (Chinese, question-ending)

```markdown
标题：转发代理 SSE 响应按 ~4KB 缓冲窗口聚合下发，建议逐事件 flush

在使用 Resin 转发代理代理 `text/event-stream`（SSE）响应时，事件到达客户端
会被合并成约 4KB 的批次：慢速或小体积事件流会滞留于 net/http 响应写缓冲，
直到填满或响应结束才下发，客户端表现为事件延迟、成批到达。

根因在 `internal/proxy/forward.go` 的 `handleHTTP`（约 294 行）：上游响应体
经裸 `io.Copy(w, resp.Body)` 拷贝，全程未调用 `Flush`。

我已在本地实现最小修复并写好测试：按 `Content-Type: text/event-stream` 判定
（`mime.ParseMediaType`，兼容 `; charset=utf-8` 等参数），在 ResponseWriter
支持 `http.Flusher` 时经"每次 Write 后 Flush"的包装拷贝；非 SSE 响应保持原
`io.Copy` 路径逐字节不变，不支持 Flusher 时安全回退。测试覆盖逐事件到达
（httptest 真实服务端 + 门控上游体）、每次写后 flush 交错、非 SSE 不变性、
无 Flusher 回退，`go build ./internal/proxy/` 与 `go test ./internal/proxy/` 相关用例全绿。

该修复属通用正确性、不含任何产品特异逻辑。请问上游是否接受此方向？如欢迎
愿发 PR？
```

## 5. User checklist (external actions — user executes)

1. Post the probe issue (§4) to `github.com/Resinat/Resin` issues.
2. Await maintainer response.
3. Welcomed → open PR with the patch (§1) + PR body (§3), commit message
   `forward: flush SSE responses per event`.
4. Rejection, or stall beyond one upstream release cycle (upstream cuts on a
   roughly monthly cadence) → dormant patch-fork trigger per D-002: fresh
   repo (not a GitHub fork) + main rebasing upstream tags + copied release.yml
   (byte-identical asset names) + manifest `repo:`/`version:` repoint +
   `THIRD_PARTY.md` fork line. Day-1 infra is NOT pre-built (D-002.6).
5. If merged upstream → the `mode-b-sse-buffering` exemption removal condition
   evaluates per D-002.5 (shipped engine implements per-event flush).
