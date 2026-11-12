# Performance hillclimb

## Row reuse across searches, 2026-11-10

Compared uncommitted row-cache changes against `233fed9`. List rows were reused
only when present in the immediately preceding result window. Narrowing and then
broadening a query discarded and rebuilt recent widgets. A 128-row working set
now retains them across queries. Row identity includes view, entry ID,
update time, relative age, pin state and displayed label. Including the label
prevents same-timestamp alias edits from reusing stale widgets. Config and favicon rerenders clear the
cache; cached rows have selection styling cleared before reattachment.

Three fresh release processes per version, alternating before/after,
after/before, before/after. Each uses the same synthetic history and a private
Weston with pixman and GTK Cairo rendering. No builds ran during timing. Values
below are medians of each run's longest GTK dispatch in that scenario, not p95
or p99 of all dispatches. At 40 ms between typed characters:

- JSON query: 24.65 to 13.96 ms, 43% lower.
- Log query: 24.72 to 14.11 ms, 43% lower.
- Code query: 24.49 to 14.56 ms, 41% lower.
- Large-payload query: 26.63 to 14.01 ms, 47% lower.
- Broad one-character query: 11.05 to 10.89 ms.

Initial result-paint latency is mostly unchanged. First-use language highlighting
still stalls: 50.05 to 51.67 ms. Image preview was 23.75 to 24.48 ms, navigation
24.41 to 25.97 ms. These small differences do not establish improvements or
regressions with three observations. The main demonstrated gain is repeated
typing through recently seen rows, with a memory tradeoff of retaining up to
128 widgets in addition to any live rows not in that working set. Native Niri/GPU
presentation, disk-cold behavior and production-scale memory impact were not measured.

The runtime harness also verifies row object reuse across an empty query result,
bounded cache size, and replacement after forced rerender. Existing clipboard,
secret search/rename/delete, stale preview and OCR contention flows pass.
Focused UI checks, strict Clippy, changed-file formatting and release build pass.

Two candidates were rejected after measurement: removing the 8 ms debounce and
shrinking the initial page. Both caused broad-query/typing or navigation
regressions. Their runtime changes were removed. No installed binary or hotkey
configuration was changed. Measurements were taken before committing this follow-up.

Full samples, medians and executable SHA-256 hashes are in
[PERFORMANCE-ROW-CACHE.json](PERFORMANCE-ROW-CACHE.json). Build and copy the
`headless_ui_perf` executable for each revision before switching source trees.
The comparison script refuses identical executable hashes:

```sh
python3 scripts/compare-ui-performance.py /path/to/before-ui-bench /path/to/after-ui-bench /tmp/rsclip-comparison --rounds 3
```

## Earlier measurements

Measured 2026-11-07. Starting revision `a3b3534` on `master`, three commits behind `origin/master`. Existing OCR work was preserved. No commit, push, installation, live clipboard change, or desktop configuration change was performed.

The optimized source constructs the UI about five times faster and paints search results about 2.5 times faster in the controlled headless comparison. It does not eliminate every hitch. First-use syntax highlighting and GTK layout still exceed a frame budget.

## Before and after

Three fresh processes per binary, alternated in the order baseline, optimized, optimized, baseline, baseline, optimized. No compilation ran alongside these measurements. The baseline executable was frozen before application edits, with the same startup/search timing instrumentation. Each process seeds an identical 1,606-entry synthetic history and uses a private Weston with pixman and GTK's Cairo renderer. Search covers a large single-line payload, a selective text query, a no-match query, links, and a broad one-character query.

Values are medians. Startup has three observations per binary. Search has fifteen observations per binary, five queries in each of three processes. These samples do not support a reliable p99 claim.

| Measurement | Baseline | Optimized | Reduction |
|---|---:|---:|---:|
| UI construction | 28.39 ms | 5.52 ms | 80.6% |
| First after-paint from GTK initialization | 80.24 ms | 49.09 ms | 38.8% |
| First populated history from GTK initialization | 88.56 ms | 51.50 ms | 41.8% |
| Worst startup dispatch in each run | 44.67 ms | 34.24 ms | 23.3% |
| Query to results applied | 72.95 ms | 14.32 ms | 80.4% |
| Query to first results paint | 100.25 ms | 40.78 ms | 59.3% |
| First-use code-language scenario | 53.11 ms | 50.65 ms | Effectively unchanged |
| Navigation's worst dispatch | 26.43 ms | 26.59 ms | Effectively unchanged |

All three UI-construction observations improved: baseline range 27.14–30.57 ms, optimized 5.10–5.68 ms. First-history population ranged 78.09–90.56 ms before and 50.10–52.38 ms after. Search-to-paint ranged 90.60–102.32 ms before and 31.21–54.59 ms after.

Raw samples, exploratory rounds, binary SHA-256 hashes, and the contention-test output are in [PERFORMANCE-RESULTS.json](PERFORMANCE-RESULTS.json). Frozen executables and full logs remain under `/tmp/rsclip-hillclimb/` for this session.

## Hillclimb rounds

Each exploratory round below was a single release run, followed by the stronger alternating comparison above. Individual round timings include machine noise and are not independent estimates of each change's contribution.

| Round | Change | Observation | Decision |
|---|---|---|---|
| 1 | Initial list uses viewport plus lookahead, retaining the larger scrolling window | Startup dispatch 29.04 ms; navigation checks passed | Keep |
| 2 | Debounce 60 to 20 ms | Five-query median paint 55.56 ms | Keep provisionally |
| 3 | Dedicated preview reader; database initialization and OCR database work off GTK | Warm-query timing similar; real worker-content/stale-result/OCR-lock checks passed | Keep for I/O isolation |
| 4 | Create GtkSourceView on first code selection | UI construction 5.57 ms | Keep |
| 5 | Debounce 20 to 8 ms | Five-query median paint 41.45 ms | Keep |
| 6 | Cancel obsolete SQLite reads through a shared generation and progress handler | Forced slow scan stopped in 108.28 ms with a 1,000-operation interval | Refine interval |
| 7 | Check cancellation every 100 SQLite VM operations | Same forced slow-scan test stopped in 6.60 ms; subsequent writes and reads passed | Keep |
| 8 | Remove forced highlighting of the language keepalive buffer | Markdown and HTML still blocked synchronously for about 50 ms | Reject; restore original warming behavior |

The cancellation timings use an intentionally slow SQLite LIKE function, not a normal history query. The uncancelled fixture would spend over a second evaluating predicates. This verifies early cancellation and connection reuse rather than predicting real query latency.

The prior trigram experiment is also not included in the implementation. It sped up selective/no-match searches but regressed queries matching most rows. Search matching, ordering, filters, and the searchable text prefix retain their existing semantics.

## Implementation

- The list worker now performs the initial SQLite open/migrations. GTK creates and presents the shell without waiting for that connection. Preview and copy connections open lazily after an entry is available.
- Text previews read their bounded body on a dedicated persistent thread. That queue coalesces pending selections; GTK applies a result only while its entry identity and preview generation still match.
- UI state no longer owns a SQLite connection. OCR reads, recognition, saves, and rereads execute on a worker. Completion updates GTK only if the relevant selection is still active.
- The first history page renders approximately 40 rows under the fixture's viewport estimate, rather than approximately 120. Scrolling and selection reloads retain their existing larger window and row reuse.
- The code-preview widget/buffer is created once, on demand, and reused thereafter. Existing preview text/line/highlighting limits remain.
- Search uses an 8 ms debounce. An atomic generation lets an in-flight stale read abort between SQLite VM batches. Its cancellation callback is removed before the next ordered mutation. Clipboard copies and list-worker writes retain their existing order.

The SQLite `hooks` feature enables cooperative cancellation. The UI's added rusqlite dependency is for the cancellation test's controlled SQL function. No schema migration or new search index was added. A pre-existing nested conditional in list rendering was collapsed to satisfy the strict Clippy check.

## Behavior and contention checks

The optimized shell painted at 45.49 ms from GTK initialization while an old-schema fixture's migration waited behind a real SQLite write lock. UI construction took 5.35 ms and the longest startup dispatch was 16.04 ms in that run. History remained unloaded while the lock was held and populated after release. This tests a real lock, not a mocked delay.

The headless harness also checks delivery of a bounded full preview, rejection of an obsolete response for the same entry, secrets search/rename/delete, and search while an OCR save waits behind a held database write lock. OCR completion preserves the user's later selection. OCR uses a synthetic command for this test, so this does not validate recognition accuracy or the existing Indra work.

The existing scenarios still exercise fast/slow typing, long text and code, key navigation, image preview, favicon refresh, complete delivery of a 5 MB clipboard payload, ordered clipboard writes, and cancellation of a paste belonging to an older copy or overlay session.

## Limits and remaining work

This is fresh-process GTK startup with warm filesystem caches after fixture seeding. Timing starts at `gtk::init`, so it excludes process creation, dynamic loading before main, and compositor/D-Bus activation. An after-paint callback is not proof of physical monitor presentation. The headless compositor lacks layer-shell and emitted the expected layer-shell and missing-session-bus warnings. The installed native Niri/GPU path was not replaced or timed.

Cold highlighting remains a material problem. The paired release runs measured about 50 ms. An exploratory release run reached 88.97 ms, and the first broader debug-suite run failed the unchanged 100 ms budget with a 173.16 ms first-use code-preview outlier. The experimental removal of forced keepalive highlighting did not remove the roughly 50 ms synchronous work and was reverted. Per-language diagnostics were retained. This is not a claim of zero hitches, and the outliers are not removed from the record.

The final default-member debug suite passed 112 core tests, 10 daemon tests, 47 UI unit tests, and the headless GTK integration test. Its first-use code-preview dispatch reached 55.54 ms. Strict UI Clippy passed with warnings denied. This passing rerun does not resolve the earlier highlighting outlier. The optional Indra crate was outside this test command's default members.

GTK layout/paint still takes roughly 20–30 ms in several scenarios. Favicon cache misses and file availability checks still perform synchronous filesystem work. Exact counts still precede delivery of full search pages. Ordered writes can still delay queries on the list worker, and running image decodes are not cancelled. These are the next targets if further work is wanted; another blanket increase in thread count will not fix GTK layout or grammar setup.

CPU use during sustained typing, large-history index alternatives, disk-cold startup, real desktop shortcut-to-frame time, and slow network filesystems were not benchmarked. Native clipboard ownership and automated paste were not exercised; the harness uses a private clipboard writer.

## Reproduce

```sh
RSCLIP_REQUIRE_HEADLESS_UI=1 cargo test --release -p rsclip-ui --test headless_ui_perf -- --nocapture
RSCLIP_TEST_STARTUP_LOCK=1 RSCLIP_REQUIRE_HEADLESS_UI=1 cargo test --release -p rsclip-ui --test headless_ui_perf -- --nocapture
cargo test -p rsclip-ui --bin rsclip events::tests -- --nocapture
RSCLIP_REQUIRE_HEADLESS_UI=1 cargo test --locked
cargo clippy -p rsclip-ui --all-targets --locked -- -D warnings
```

## Completion record

| Result | Source/runtime paths | Check | State | Evidence |
|---|---|---|---|---|
| Repeatable baseline | Headless UI harness on private Weston | Three alternating release pairs | proved | Raw JSON and frozen binaries |
| Startup and search improvements | UI window, events, previews, list | Same harness for each candidate | proved | Comparison and round tables above |
| Behavior preservation | History/secrets, selection, writes/copies | Unit and headless integration tests | proved within tested boundaries | 169 unit tests and headless GTK integration passed; native limitations below |
| Final comparison | This report and raw results | Inspect exact diff, formatting, strict Clippy | complete | Paired release results retained; earlier highlighting failure disclosed |

## Surface ledger

| Surface | Applies | Required check | State |
|---|---|---|---|
| Cold show/toggle and hidden preload | applies | Cold show, hide/show, and locked migration exercised; preload code inspected | Native shortcut/preload latency unmeasured |
| History filters, sorting, scrolling, selection | applies | Core query tests, 5k-row window simulation, real GTK navigation | Passed in focused checks |
| History and secret search | applies | Results, stale reads/responses, latency, secrets rename/delete | Passed in release runs |
| Clipboard copy/paste, pin/delete/rename | applies | Ordered writes/copies and session checks | Private writer passed; native ownership/paste untested |
| Text/code/image/file previews | applies | Bounded text, stale upgrade, code-language switching, image decode | File preview implementation unchanged; slow mounts untested |
| Daemon notifications and config reload | applies | Producers/consumers inspected; shared types compile | Notification/config runtime not separately exercised |
| SQLite schema and CLI list commands | applies | Schema unchanged, old-schema readiness, existing core/daemon tests | Focused checks passed |
| Network, browser, mobile, remote clients | does not apply | No such search/presentation path | proved |
| Existing OCR implementation | applies | Preserve user work; exercise asynchronous result save under lock | Synthetic backend passed; recognition accuracy untested |
| Docs and performance contracts | applies | Record achieved results, failures, and untested limits | This report and raw JSON |

## Hard constraints

Preserve unrelated work. Do not commit or push. Do not edit installed binaries or live configuration. Do not print or copy clipboard contents or secret values. Keep GTK objects on the main thread. Keep clipboard copies and database mutations ordered. Retain existing search/filter/sort semantics and preview bounds. Builds alone do not prove runtime responsiveness.
