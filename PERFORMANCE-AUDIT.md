# Startup and search performance audit

Initial audit on 2026-11-07 at local source revision `a3b3534`, with existing uncommitted OCR work preserved. This records the code before optimization. For the subsequent implementation and measured comparisons, see [the hillclimb results](PERFORMANCE-RESULTS.md). The installed application has not been replaced.

## Recommendation

Keep GTK for now. Remove synchronous database and filesystem work from its main thread, make obsolete searches cancellable, deliver the first results before computing an exact count, and reduce first-frame widget work. Evaluate trigram indexing after those changes, using the actual search semantics and query distribution.

More threads can isolate slow work, but cannot guarantee zero missed frames. GTK widgets, text layout, and the current GtkSourceView highlighting path still need bounded time on the main thread. Optimize both input-to-result latency and longest main-thread task. A responsive search field showing old results for a second is still a slow search.

## Scope and constraints

- Preserve the existing dirty OCR files, history, clipboard, and resident desktop processes.
- Do not commit or push. Do not change installed binaries or desktop configuration.
- Run UI checks in the existing private headless compositor harness. Read live history only for aggregate counts and query timings. Do not print or copy clipboard contents or secret values.
- Audit the code in this checkout. The branch is three commits behind its remote tracking branch; this is not a review of those additional commits or proof about the installed binary.

Applicable paths include cold show/toggle, hidden preload, resident reopen, history and secrets search, all history filters/sorts, scrolling, previews, daemon notifications, clipboard writes, OCR completion, configuration reload, and both CLI list commands. Browser, mobile, and network search paths do not apply. The UI does not fetch favicons over the network.

## What already runs concurrently

`app.rs` routes show, toggle, preload, and quit through a resident `GtkApplication`. `window.rs::build_ui` creates the runtime on first use. Subsequent shows reuse that runtime.

`events.rs::start_workers` creates an ordered list/write worker and a separate ordered copy worker. Each owns a SQLite connection. List requests waiting in the queue collapse to the newest request, writes keep their order, and GTK rejects results with an obsolete generation or changed search context. Worker completion wakes the main loop directly rather than waiting for a polling timer.

Image preview reads and decoding use `gio::spawn_blocking`, then apply the result only to the matching preview generation. There is a six-image cache with a maximum decoded edge of 1600 pixels. OCR inference also runs on a GIO worker. The daemon watches clipboard changes separately and has a favicon worker.

These are useful existing boundaries. Adding a thread per keystroke would lose the benefits of coalescing, add competing scans, and increase CPU and memory pressure.

## Startup

Source: `crates/rsclip-ui/src/window.rs:88`, `crates/rsclip-ui/src/components/preview.rs:47`, `crates/rsclip-ui/src/highlight.rs:65`, `crates/rsclip-core/src/db/mod.rs:26`.

Before showing a window, `build_ui` discovers and creates paths, loads configuration, opens SQLite and runs migrations, loads CSS, constructs both preview channels including GtkSourceView, installs socket/config listeners, and realizes the window. All of that happens on the GTK thread. The list worker then opens another database connection independently.

For schema version 5, migration is just a version check. For an older database, migration can backfill text classification or rebuild the entries table. That is a large potential first-launch delay, not a demonstrated cost on the current version-5 history. SQLite also has a five-second busy timeout. An open or write that encounters contention is unsuitable for the GTK thread.

The preview panel creates a source buffer and resolves the style scheme even if the user initially needs only the search field and history list. First scheme setup writes an XML file into a temporary directory and forces a scheme rescan. Language compilation is warmed one language every 250 ms while hidden, but first opening the UI can interrupt warmup. `realize()` creates a native surface; it does not establish that a rendered frame has reached the monitor or that every renderer/font cache is warm.

The running desktop already has `rsclip preload` and `rsclipd watch`. Recommending preload alone would miss this setup. A delay on an ordinary shortcut press can therefore be resident presentation, resetting the query, list replacement, or preview work rather than process startup.

Recommended changes:

1. Build the search/list shell without waiting for database initialization. Show an explicit loading state, then publish a usable first page from a background-owned connection. Start configuration loading and database readiness work early; keep GTK object creation on GTK.
2. Separate schema preparation from ordinary connection setup. Serialize migrations and publish readiness before other workers start using the database. Preserve migration correctness across the UI and daemon starting together.
3. Create the source preview lazily. Prioritize a plain preview and first frame. Keep resident preload, but measure shortcut-to-frame on the real compositor before deciding to add any offscreen rendering warmup.
4. Initially instantiate only the rows needed for the viewport. Current browse windows include 100 padding rows beyond the visible-row estimate. Fetching them can be cheap while building and laying out all their widgets still costs a frame.

## Search

Source: `crates/rsclip-ui/src/events.rs:163`, `:262`, `:372`; `crates/rsclip-core/src/db/entries.rs:171`, `:462`.

The search entry has its GTK search delay explicitly disabled. The application adds a single 60 ms debounce. After that, the worker loads the first page and, when the page is full, computes the exact total before sending any results. A common query therefore waits for a count across all matches even when the top rows were already available.

Search uses `LIKE '%query%'` over titles, previews, the bounded text side table, link URL/domain, colors, and OCR text. The text side table prevents scanning complete multi-megabyte payloads, but this remains scan-based substring search. Ordinary B-tree indexes do not solve the leading wildcard. Exact count and sparse/no-match pages become more expensive as searchable history grows.

The worker coalesces queued searches but does not cancel an already executing query. If a query takes a long time, a newer query waits for it even though its eventual result will be discarded. Database writes also share that worker, so a slow write or lock wait can hold up new searches without blocking GTK itself.

Recommended sequence:

1. Publish the first page with a pending count or `has_more`, then send an exact count separately if the same generation is still active. Use a short read transaction or revision tagging when page/count consistency matters. Update scrolling so it does not require an exact total before results appear.
2. Add cooperative SQLite cancellation using a generation-aware progress handler, or a carefully scoped interrupt handle. Cancel only reads made obsolete by a newer context. Never interrupt an ordered write or treat an expected cancellation as a user-facing error. Check for obsolescence between page and count too.
3. Try a shorter debounce only after cancellation and render costs are controlled. The fixed 60 ms is a latency floor, but removing it immediately can multiply scans and list updates.
4. Prototype an FTS5 trigram candidate index for selective substrings. Preserve the existing filter/sort rules and verify candidate matches with the current predicates. Keep a scan path for one/two-character and very common queries.

A word-token FTS index is not a drop-in replacement. Current matching includes substrings in URLs, code punctuation, colors, and OCR. Whitespace is trimmed, matching follows SQLite LIKE rules, and `%` and `_` currently act as wildcards. Changing any of those requires an explicit product decision. Text-body search currently uses only the first 65,537 characters; indexing complete payloads would expand existing behavior and cost.

Maintain an index for every relevant insert/update, soft deletion, OCR replacement, and migration across all writers. Do not accidentally index secret values. Secrets currently search aliases only. Trigram indexing also adds storage and ingestion work, so benchmark capture latency and migration time alongside query latency.

## Remaining main-thread stalls

- **Text preview reads.** `preview.rs::with_preview_text` calls SQLite directly. The deferred large-preview upgrade uses `idle_add_local_once`, which still runs on GTK. Move bounded reads, sanitization, and language detection to a dedicated latest-request worker. Return immutable preview data, and apply it only if entry identity and preview generation still match. Do not put this behind a long search/count on the same queue.
- **First-use highlighting.** `highlight.rs::retain_language_contexts` calls `ensure_highlight` synchronously. Idle scheduling does not make this preemptible. Start with plain text when highlighting is cold. Continue using caps and cached contexts. If first-use compilation remains too slow, evaluate an off-thread tokenizer returning a bounded list of styled spans; do not move GTK buffers into arbitrary worker threads.
- **Favicon cache misses.** `list.rs::cached_favicon` synchronously opens and decodes a PNG for each uncached domain. Display the fallback immediately and decode asynchronously with bounded concurrency. Deduplicate requests and invalidate only changed domains where possible.
- **File previews.** `preview.rs::render_file_preview` runs `path.exists()` calls on GTK. A bounded number of calls can still block on a slow mount. Return file availability later from a worker, without delaying the preview.
- **OCR completion.** Inference is off-thread, but the initial full entry read, result save, and reread in `actions/ocr.rs` are synchronous. Move the full database portion off GTK while retaining the existing selection guard and ordered refresh behavior.
- **Automated paste.** `window.rs::close_overlay_and_paste` runs the paste tool from a GTK timer; the helper waits for `wtype` to exit. Spawn and reap it off-thread while preserving clipboard serial/session checks and ordering relative to later copies.
- **Secrets.** The list query loads complete secret values. Masking already uses only the last four characters, so its displayed output is bounded. Current secrets are small; bounded list summaries would address unusually large secret values, rather than a measured bottleneck in the current history.
- **List application.** Row reuse and virtual windows already exist. Arbitrary query changes still detach/reinsert rows in one callback. Build the visible portion first and schedule remaining work within a small time budget, or evaluate `GtkListView` factory recycling if profiling still identifies row layout as the dominant cost.

Use a bounded preview/image job queue with deduplication and latest-request priority. Generation checks currently prevent stale image display, but already queued decoding still consumes CPU. Keep clipboard copies ordered on their existing worker. Keep database writes ordered; if later separated from search reads, attach a write acknowledgement/revision to refreshes so a refresh cannot race ahead of its mutation.

## Measurements and their limits

The four `rsclip-core` performance tests passed. They cover query plans, query budgets, independence from large legacy payloads, and ingestion. Their timing helper uses the fastest of three samples. This is useful for regression detection but hides scheduler noise, first-use costs, and tail latency.

The existing headless GTK regression test also passed in the debug profile. It used a private Weston with pixman and GTK's Cairo renderer, approximately 1,600 synthetic rows, a large generated image, and pathological text payloads. Its existing default dispatch budget is 100 ms. Typing scenarios reached 28.76 ms, navigation 24.78 ms, and image preview 24.04 ms. Passing that test does not establish frame-perfect behavior.

The optimized release run passed too. Its worst startup dispatch was 42.67 ms, first-use code-language scenario 53.50 ms, typing scenario 27.02 ms, navigation 29.65 ms, and image preview 24.37 ms. UI construction took 32.69 ms, first after-paint arrived at about 84 ms, and first populated history at about 93 ms from `gtk::init`. Those stalls survive release optimization. The language scenario includes synchronous preview work and following main-loop dispatches; this does not attribute its entire duration to language compilation alone.

Added instrumentation measured startup from immediately before `gtk::init`, after fixture creation. In the debug run, GTK initialization took 6.62 ms, application registration 24.07 ms, UI construction 31.22 ms, and synchronous show 5.73 ms. The first after-paint signal arrived at about 85 ms, first populated history at about 98 ms, and worst startup dispatch was 56.47 ms. These are single-run observations. They exclude process creation, dynamic loading before main, real D-Bus activation, and disk-cold loading. The fixture had just been seeded. Weston lacks layer-shell, and an after-paint signal does not prove monitor presentation. A missing session bus generated a warning. Do not extrapolate this to the live Niri/GPU path.

A read-only aggregate inspection found 6,976 active history entries, 3,632 text-side-table rows containing about 3.08 MB in total, and 18 OCR results. The largest recorded payload was about 41.8 MB. No clipboard contents or secret values were printed or copied.

On that live history, a Python/system-SQLite probe using the current count predicate observed a 159.94 ms first common-query sample and a 4.50 ms median over 12 samples. The code-shaped query's first sample was 38.66 ms and median 9.25 ms; a no-match query's median was 7.34 ms. Compilation was running concurrently. These are exploratory tail-latency observations, not controlled cold-disk measurements or timings of the complete Rust UI path.

An isolated in-memory SQLite 3.54.0 experiment compared LIKE counts over generated bodies with the same LIKE counts over an FTS5 trigram table. Match counts agreed for all tested cases. At 70,000 rows, selective `handler_1234` counts measured 61.96 ms by scan and 6.84 ms by trigram; no-match counts measured 47.45 ms and 0.47 ms. A `github` query present in every body measured 34.69 ms by scan and 162.29 ms by trigram. Compilation was concurrent, these are medians of eight samples, and this simplified single-body query excludes production joins, ordering, paging, writes, and index backfill. It supports testing adaptive routing; it does not establish a production speedup.

## Acceptance criteria for the optimization work

Measure process launch to focused search, shortcut to presented frame, first usable page, final-keystroke to current results, preview readiness, and maximum/p95/p99 main-thread dispatch duration separately. Track memory and CPU too. A 60 Hz frame is 16.7 ms; at 120 Hz it is 8.3 ms. Aim for application callbacks below 4 ms where possible, leaving time for layout and paint. These are targets, not achieved results.

Use repeated fresh-process and resident runs, separate warm filesystem caches from true cold storage, and retain outliers. Test real layer-shell/GPU presentation on Niri, first use of each code language, slow/missing files, rapid image navigation, large secrets, database write contention, OCR completion during typing, and capture bursts. Add an old-schema startup fixture and test simultaneous daemon/UI startup. At larger histories, cover common, selective, wildcard, Unicode, one/two-character, and no-match queries across every filter and sort, history and secrets, and CLI output.

The immediate implementation order should be asynchronous startup/database and preview access, cancellable search with first-page delivery, smaller render batches and asynchronous cache misses, then measured adaptive indexing. A new UI framework or an in-memory duplicate of the whole history is not justified by the evidence collected here.

## Reproduce the checks

```sh
RSCLIP_PERF_REPORT=1 cargo test -p rsclip-core perf_ -- --nocapture
RSCLIP_REQUIRE_HEADLESS_UI=1 cargo test -p rsclip-ui --test headless_ui_perf -- --nocapture
RSCLIP_REQUIRE_HEADLESS_UI=1 cargo test --release -p rsclip-ui --test headless_ui_perf -- --nocapture
```

The added harness instrumentation records startup and includes startup dispatches and first-use language scenarios in the existing budget check. It does not change application runtime behavior. Focused Rust formatting and `git diff --check` passed. Workspace-wide formatting checks found existing formatting differences in `crates/rsclip-core/examples/perf_permutations.rs`; those were left untouched. Native Niri/GPU interaction, true disk-cold startup, slow filesystems, contention, OCR completion, and production-scale index correctness remain untested.
