//! Headless GTK main-thread benchmark for the rsclip overlay.
//!
//! Compiles the UI crate's own modules into this example (via `#[path]`), builds
//! the real window (`window::build_ui`) against a Wayland display, and drives
//! the real handlers (search entry -> debounce -> worker -> list render ->
//! preview, key navigation, copy, favicon refresh) while timing every GLib
//! main-loop dispatch on the GTK thread. Unlike `tests/headless_ui_perf.rs`,
//! which asserts budgets on a synthetic history, this reports numbers for a
//! real history.
//!
//! Run it against a copy of your history on a headless compositor:
//!
//!   B=/tmp/rsclip-bench
//!   mkdir -p $B/{state/rsclip,data/rsclip,config/rsclip,runtime}; chmod 700 $B/runtime
//!   sqlite3 -readonly ~/.local/state/rsclip/rsclip.db ".backup $B/state/rsclip/rsclip.db"
//!   ln -sfn ~/.local/share/rsclip/images $B/data/rsclip/images
//!   XDG_RUNTIME_DIR=$B/runtime weston --backend=headless --socket=rsclip-bench --idle-time=0 &
//!   env WAYLAND_DISPLAY=rsclip-bench XDG_RUNTIME_DIR=$B/runtime XDG_STATE_HOME=$B/state \
//!       XDG_DATA_HOME=$B/data XDG_CONFIG_HOME=$B/config GDK_BACKEND=wayland \
//!       cargo run --release -p rsclip-ui --example headless_search_perf
//!
//! `--idle-time=0` matters: an idle weston stops repainting, and frames then
//! drop out of the measurements. The example refuses to run unless both
//! `XDG_STATE_HOME` and `WAYLAND_DISPLAY` contain "bench", so it never opens
//! the live history and its copy step never reaches the desktop clipboard.
//!
//! Optional args: `--queries "fn main,http,a"` `--cadence-ms 150`
//! `--section typing|preview|bigtext|nav|misc|langs|all` `--verbose`.
#![allow(dead_code, unused_imports, unused_variables)]

#[path = "../src/actions/mod.rs"]
mod actions;
#[path = "../src/app.rs"]
mod app;
#[path = "../src/cli.rs"]
mod cli;
#[path = "../src/components/mod.rs"]
mod components;
#[path = "../src/config_reload.rs"]
mod config_reload;
#[path = "../src/dialogs/mod.rs"]
mod dialogs;
#[path = "../src/events.rs"]
mod events;
#[path = "../src/highlight.rs"]
mod highlight;
#[path = "../src/notify.rs"]
mod notify;
#[path = "../src/state.rs"]
mod state;
#[path = "../src/style.rs"]
mod style;
#[path = "../src/window.rs"]
mod window;

use std::rc::Rc;
use std::time::{Duration, Instant};

use gio::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use rsclip_core::models::{ClipboardEntry, EntryData, EntryFilter, EntryKind, SortMode};

use state::{AppState, RowKey};

const FRAME_MS: f64 = 16.7;

thread_local! {
    /// (phase name, instant it started) per frame-clock signal, oldest first.
    static PHASES: std::cell::RefCell<Vec<(&'static str, Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Record frame-clock phase boundaries so frame cost can be split into
/// update(CSS/animations) / layout(size-allocate) / paint(snapshot+render).
fn instrument_frame_clock(window: &gtk::ApplicationWindow) {
    use gtk::gdk::prelude::*;
    let Some(clock) = window.frame_clock() else {
        println!("(no frame clock yet)");
        return;
    };
    macro_rules! hook {
        ($connect:ident, $name:expr) => {
            clock.$connect(|_| PHASES.with(|p| p.borrow_mut().push(($name, Instant::now()))));
        };
    }
    hook!(connect_before_paint, "before-paint");
    hook!(connect_update, "update");
    hook!(connect_layout, "layout");
    hook!(connect_paint, "paint");
    hook!(connect_after_paint, "after-paint");
}

/// Drain recorded phases into (update_ms, layout_ms, paint_ms) sums.
fn take_phase_costs() -> (f64, f64, f64) {
    PHASES.with(|p| {
        let v = std::mem::take(&mut *p.borrow_mut());
        let (mut u, mut l, mut pa) = (0.0, 0.0, 0.0);
        for w in v.windows(2) {
            let d = ms(w[1].1.duration_since(w[0].1));
            // Timestamps are taken after GTK's own handler for each phase ran,
            // so the gap ending at phase X is the cost of X.
            match w[1].0 {
                "update" => u += d,
                "layout" => l += d,
                "paint" => pa += d,
                _ => {}
            }
        }
        (u, l, pa)
    })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Dispatch {
    at: Duration,
    dur: Duration,
    label: &'static str,
}

/// Snapshot used to label what a dispatch did.
fn snapshot(state: &Rc<AppState>) -> (Vec<RowKey>, Option<i64>, u64) {
    (
        state.rendered_rows.borrow().clone(),
        state.currently_previewed_entry_id.get(),
        state.preview_generation.get(),
    )
}

/// Run the main loop for `span`, timing every dispatch that did work.
fn pump(state: &Rc<AppState>, span: Duration, origin: Instant) -> Vec<Dispatch> {
    let ctx = gtk::glib::MainContext::default();
    let end = Instant::now() + span;
    let mut out = Vec::new();
    while Instant::now() < end {
        let before = snapshot(state);
        let t = Instant::now();
        if ctx.iteration(false) {
            let dur = t.elapsed();
            let after = snapshot(state);
            let label = if after.0 != before.0 {
                "list-apply"
            } else if after.1 != before.1 || after.2 != before.2 {
                "preview"
            } else {
                "other(frame/idle/timer)"
            };
            out.push(Dispatch {
                at: t.duration_since(origin),
                dur,
                label,
            });
        } else {
            std::thread::sleep(Duration::from_micros(150));
        }
    }
    out
}

fn settle(state: &Rc<AppState>) {
    // Pump until a 250 ms window passes with no long dispatch.
    let origin = Instant::now();
    for _ in 0..40 {
        let d = pump(state, Duration::from_millis(100), origin);
        if d.is_empty() {
            break;
        }
    }
    pump(state, Duration::from_millis(250), origin);
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn type_query(
    state: &Rc<AppState>,
    query: &str,
    cadence: Duration,
    verbose: bool,
) -> Vec<(String, f64, f64, f64, f64, f64)> {
    // Start each query from an empty, settled search box.
    *state.query.borrow_mut() = String::new();
    state.search_entry.set_text("");
    settle(state);
    *state.query.borrow_mut() = String::new();
    // Return rows: (prefix, list_apply_ms, preview_ms, other_max_ms, total_blocking_ms, worst_single_ms)
    let mut rows = Vec::new();
    let mut prefix = String::new();
    for ch in query.chars() {
        prefix.push(ch);
        let origin = Instant::now();
        let t = Instant::now();
        // Mimic typing: insert one char at the end of the entry.
        let mut pos = state.search_entry.text().chars().count() as i32;
        state.search_entry.insert_text(&ch.to_string(), &mut pos);
        let insert_ms = ms(t.elapsed());
        let dispatches = pump(state, cadence, origin);
        let mut list = 0.0;
        let mut prev = 0.0;
        let mut other_max = 0.0f64;
        let mut worst = insert_ms;
        let mut total = insert_ms;
        for d in &dispatches {
            let m = ms(d.dur);
            total += m;
            worst = worst.max(m);
            match d.label {
                "list-apply" => list += m,
                "preview" => prev += m,
                _ => other_max = other_max.max(m),
            }
            if verbose {
                println!(
                    "      +{:6.1}ms  {:6.2}ms  {}",
                    ms(d.at.saturating_sub(Duration::ZERO)) - 0.0,
                    m,
                    d.label
                );
            }
        }
        rows.push((prefix.clone(), list, prev, other_max, total, worst));
    }
    // let the last keystroke fully settle
    let origin = Instant::now();
    let tail = pump(state, Duration::from_millis(400), origin);
    let mut list = 0.0;
    let mut prev = 0.0;
    let mut other_max = 0.0f64;
    let mut worst = 0.0f64;
    let mut total = 0.0;
    for d in &tail {
        let m = ms(d.dur);
        total += m;
        worst = worst.max(m);
        match d.label {
            "list-apply" => list += m,
            "preview" => prev += m,
            _ => other_max = other_max.max(m),
        }
    }
    if let Some(last) = rows.last_mut() {
        last.1 += list;
        last.2 += prev;
        last.3 = last.3.max(other_max);
        last.4 += total;
        last.5 = last.5.max(worst);
    }
    rows
}

fn print_rows(title: &str, rows: &[(String, f64, f64, f64, f64, f64)]) {
    println!("\n== {title}");
    println!(
        "{:<12} {:>10} {:>10} {:>12} {:>11} {:>11}  (main-thread ms per keystroke window)",
        "typed", "list-apply", "preview", "frame/other", "total", "worst-1"
    );
    for (p, l, pr, o, t, w) in rows {
        let flag = if *w > FRAME_MS {
            "  <-- >16.7ms dispatch"
        } else {
            ""
        };
        println!("{p:<12} {l:>10.2} {pr:>10.2} {o:>12.2} {t:>11.2} {w:>11.2}{flag}");
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t = Instant::now();
    let v = f();
    (v, ms(t.elapsed()))
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let queries = arg_value(&args, "--queries").unwrap_or_else(|| "fn main,http,a,e".into());
    let cadence_ms: u64 = arg_value(&args, "--cadence-ms")
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let section = arg_value(&args, "--section").unwrap_or_else(|| "all".into());
    let verbose = args.iter().any(|a| a == "--verbose");

    let isolated =
        |name| std::env::var_os(name).is_some_and(|v| v.to_string_lossy().contains("bench"));
    anyhow::ensure!(
        isolated("XDG_STATE_HOME"),
        "refusing to run without an isolated XDG_STATE_HOME (contains 'bench')"
    );
    // The copy section hands the largest entry to wl-copy; on the desktop
    // display any client could read it.
    anyhow::ensure!(
        isolated("WAYLAND_DISPLAY"),
        "refusing to run without a private WAYLAND_DISPLAY (contains 'bench')"
    );
    gtk::init()?;
    let app = gtk::Application::builder()
        .application_id("io.github.radhey.rsclip.bench")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gio::Cancellable::NONE)?;

    let (runtime, build_ms) = time(|| window::build_ui(&app));
    let runtime = runtime?;
    let state = Rc::clone(&runtime.state);
    println!("build_ui: {build_ms:.1} ms");
    if let Some(renderer) = runtime.window.native().and_then(|native| native.renderer()) {
        println!("GSK renderer: {}", renderer.type_().name());
    }

    let (_, show_ms) = time(|| runtime.show_reset());
    instrument_frame_clock(&runtime.window);
    println!("show_reset (sync part): {show_ms:.1} ms");
    let origin = Instant::now();
    let first = pump(&state, Duration::from_millis(1500), origin);
    let mut sum = 0.0;
    println!("first-show dispatches >2ms:");
    for d in &first {
        sum += ms(d.dur);
        if ms(d.dur) > 2.0 {
            println!("   +{:7.1}ms  {:7.2} ms  {}", ms(d.at), ms(d.dur), d.label);
        }
    }
    println!(
        "first-show total main-thread busy: {sum:.1} ms; entries loaded={} total={}",
        state.entries.borrow().len(),
        state.entries_total.get()
    );
    settle(&state);

    if section == "all" || section == "typing" {
        for cadence in [cadence_ms, 40] {
            for q in queries.split(',') {
                let _ = take_phase_costs();
                let rows = type_query(&state, q, Duration::from_millis(cadence), verbose);
                let (u, l, pa) = take_phase_costs();
                println!(
                    "   [frame-clock totals over whole run: update(css/anim) {u:.1} ms, layout {l:.1} ms, paint(snapshot+render) {pa:.1} ms]"
                );
                print_rows(
                    &format!("typing {q:?} at {cadence} ms/char (real handlers)"),
                    &rows,
                );
            }
        }
    }

    if section == "all" || section == "preview" {
        preview_section(&state);
    }
    if section == "all" || section == "bigtext" {
        bigtext_section(&state);
    }
    if section == "all" || section == "nav" {
        nav_section(&state);
    }
    if section == "all" || section == "misc" {
        misc_section(&state);
    }
    if section == "all" || section == "langs" {
        language_switch_section(&state);
    }
    Ok(())
}

fn all_summaries(state: &Rc<AppState>) -> Vec<ClipboardEntry> {
    rsclip_core::Database::open(&state.db_path)
        .unwrap()
        .list_entry_summaries_page("", EntryFilter::All, SortMode::Default, 100_000, 0)
        .unwrap()
}

/// Direct timing of the pieces that run per search result / per selection.
fn preview_section(state: &Rc<AppState>) {
    println!("\n== per-selection preview render (components::preview::render_preview), sync part");
    let entries = all_summaries(state);
    let pick = |f: &dyn Fn(&ClipboardEntry) -> bool| -> std::vec::IntoIter<ClipboardEntry> {
        entries
            .iter()
            .filter(|e| f(e))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    };
    let mut cases: Vec<(&str, ClipboardEntry)> = Vec::new();
    let is_text = |e: &ClipboardEntry| matches!(e.data, EntryData::Text);
    if let Some(e) = pick(&|e| is_text(e) && e.size_bytes < 400).next() {
        cases.push(("tiny text", e));
    }
    if let Some(e) = pick(&|e| is_text(e) && (2_000..8_000).contains(&e.size_bytes)).next() {
        cases.push(("text ~2-8KB", e));
    }
    if let Some(e) = pick(&|e| is_text(e) && (8_000..64_000).contains(&e.size_bytes)).next() {
        cases.push(("text 8-64KB", e));
    }
    if let Some(e) = pick(&|e| is_text(e)).max_by_key(|e| e.size_bytes) {
        cases.push(("largest text (legacy)", e));
    }
    for (i, e) in pick(&|e| is_text(e) && e.size_bytes > 65_536)
        .take(3)
        .enumerate()
    {
        cases.push((["text >64KB #1", "text >64KB #2", "text >64KB #3"][i], e));
    }
    if let Some(e) =
        pick(&|e| matches!(e.data, EntryData::Image { .. })).max_by_key(|e| e.size_bytes)
    {
        cases.push(("largest image (cold)", e));
    }
    if let Some(e) = pick(&|e| matches!(e.data, EntryData::Image { .. })).nth(3) {
        cases.push(("image (cold)", e));
    }
    if let Some(e) = pick(&|e| matches!(e.data, EntryData::Link { .. })).next() {
        cases.push(("link", e));
    }
    if let Some(e) = pick(&|e| matches!(e.data, EntryData::File { .. })).next() {
        cases.push(("file list", e));
    }
    if let Some(e) = pick(&|e| matches!(e.data, EntryData::Color { .. })).next() {
        cases.push(("color", e));
    }
    // code entries by language
    for e in pick(&|e| {
        is_text(e)
            && rsclip_core::syntax::detect_code_language(e.preview_text.as_deref().unwrap_or(""))
                .is_some()
    })
    .take(1)
    {
        cases.push(("code (detected)", e));
    }
    println!(
        "{:<24} {:>10} {:>12} {:>14}",
        "case", "size", "render sync", "next frames(100ms)"
    );
    for (name, e) in cases {
        state::invalidate_preview_cache(state);
        let (_, sync_ms) = time(|| components::preview::render_preview(state, &e));
        let origin = Instant::now();
        let d = pump(state, Duration::from_millis(150), origin);
        let frames: f64 = d.iter().map(|d| ms(d.dur)).sum();
        let worst = d.iter().map(|d| ms(d.dur)).fold(0.0, f64::max);
        println!(
            "{:<24} {:>10} {:>10.2}ms {:>9.2}ms (worst {:.2})",
            name, e.size_bytes, sync_ms, frames, worst
        );
        // second time is warm (image cache etc.)
        state::invalidate_preview_cache(state);
        let (_, warm) = time(|| components::preview::render_preview(state, &e));
        pump(state, Duration::from_millis(100), origin);
        println!("{:<24} {:>10} {:>10.2}ms (2nd render)", "", "", warm);
    }

    println!("\n== row construction cost (components::list::entry_row), per row");
    let sample: Vec<_> = entries.iter().take(200).collect();
    let (_, t) = time(|| {
        for e in &sample {
            let _ = components::list::entry_row(e, &state.favicon_icon_dir);
        }
    });
    println!(
        "entry_row x{}: {:.2} ms total, {:.3} ms/row (widgets only, not yet in a parent)",
        sample.len(),
        t,
        t / sample.len() as f64
    );
}

fn nav_section(state: &Rc<AppState>) {
    println!("\n== keyboard navigation (actions::selection::move_selection), 60 presses @ 30ms");
    *state.query.borrow_mut() = String::new();
    state.search_entry.set_text("");
    settle(state);
    let mut per = Vec::new();
    for _ in 0..60 {
        let origin = Instant::now();
        let (_, sync) = time(|| actions::selection::move_selection(state, 1));
        let d = pump(state, Duration::from_millis(30), origin);
        let total = sync + d.iter().map(|d| ms(d.dur)).sum::<f64>();
        per.push(total);
    }
    per.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "per-press main-thread ms: p50={:.2} p90={:.2} p99={:.2} max={:.2}",
        percentile(&per, 0.5),
        percentile(&per, 0.9),
        percentile(&per, 0.99),
        per.last().unwrap()
    );
}

fn misc_section(state: &Rc<AppState>) {
    println!("\n== refresh / rerender paths (sync GTK-thread part + following dispatches)");
    let origin = Instant::now();
    let (_, t) = time(|| actions::refresh::rerender_current_list(state));
    let d = pump(state, Duration::from_millis(300), origin);
    println!(
        "rerender_current_list (favicon/config reload): sync {t:.2} ms, follow-up dispatches {:.2} ms",
        d.iter().map(|d| ms(d.dur)).sum::<f64>()
    );
    components::list::clear_favicon_cache();
    let (_, t) = time(|| actions::refresh::rerender_current_list(state));
    let d = pump(state, Duration::from_millis(300), origin);
    println!(
        "rerender_current_list with cold favicon cache: sync {t:.2} ms, follow-up {:.2} ms",
        d.iter().map(|d| ms(d.dur)).sum::<f64>()
    );
    let (_, t) = time(|| actions::refresh::refresh_entries(state));
    let d = pump(state, Duration::from_millis(400), origin);
    println!(
        "refresh_entries (daemon change event, visible): sync {t:.2} ms, dispatches {:.2} ms (worst {:.2})",
        d.iter().map(|d| ms(d.dur)).sum::<f64>(),
        d.iter().map(|d| ms(d.dur)).fold(0.0, f64::max)
    );
    let origin = Instant::now();
    let (_, t) = time(|| actions::refresh::rerender_link_rows(state));
    let d = pump(state, Duration::from_millis(300), origin);
    println!(
        "rerender_link_rows (favicon fetched): sync {t:.2} ms, follow-up {:.2} ms",
        d.iter().map(|d| ms(d.dur)).sum::<f64>()
    );
    // Enter / Ctrl+C on the largest entry: the read and wl-copy run on the
    // copy worker; the GTK thread only queues the request.
    let all = all_summaries(state);
    if let Some(big) = all.iter().max_by_key(|e| e.size_bytes) {
        let origin = Instant::now();
        let (_, t) = time(|| {
            actions::clipboard::queue_entry_copy(state, big.id, state::AfterCopy::REPORT_ENTRY)
        });
        let d = pump(state, Duration::from_millis(1_000), origin);
        println!(
            "copy largest entry ({} bytes): sync {t:.2} ms, worst following dispatch {:.2} ms",
            big.size_bytes,
            d.iter().map(|d| ms(d.dur)).fold(0.0, f64::max)
        );
    }
}

/// A full (non-summary) text entry, so the preview uses `text` as is.
fn synthetic_text_entry(id: i64, text: String) -> ClipboardEntry {
    ClipboardEntry {
        id,
        content_hash: String::new(),
        kind: EntryKind::Text,
        mime_type: "text/plain".into(),
        title: "synthetic".into(),
        preview_text: Some(text.chars().take(200).collect()),
        size_bytes: text.len() as i64,
        text_content: Some(text),
        pinned: false,
        copied_at: 0,
        updated_at: 0,
        last_used_at: None,
        use_count: 0,
        data: EntryData::Text,
    }
}

/// Large text previews: sync render plus the layout/paint that follows.
fn bigtext_section(state: &Rc<AppState>) {
    println!("\n== large text preview: sync render vs following frames (TextView layout)");
    let cases = [
        ("64KB single-line base64", "QUJDRA==".repeat(8 * 1024)),
        (
            "64KB JSON 1300 lines",
            "  {\"key\": \"value\", \"number\": 12345, \"flag\": true},\n".repeat(1_300),
        ),
        (
            "64KB log 1100 lines",
            "2026-09-30T12:00:00 INFO request served in 3 ms by worker 7\n".repeat(1_100),
        ),
    ];
    for (index, (name, text)) in cases.into_iter().enumerate() {
        let entry = synthetic_text_entry(-1 - index as i64, text);
        for pass in ["cold", "warm"] {
            state::invalidate_preview_cache(state);
            let _ = take_phase_costs();
            let origin = Instant::now();
            let (_, sync_ms) = time(|| components::preview::render_preview(state, &entry));
            let d = pump(state, Duration::from_millis(2500), origin);
            let (_, l, pa) = take_phase_costs();
            let worst = d.iter().map(|d| ms(d.dur)).fold(0.0, f64::max);
            let total: f64 = d.iter().map(|d| ms(d.dur)).sum();
            println!(
                "{name:<26} {pass}: sync {sync_ms:6.2} ms | main-thread busy after {total:8.2} ms (worst dispatch {worst:8.2}) | frame clock: layout {l:.1} paint {pa:.1}"
            );
        }
    }
}

/// Code previews alternating between languages, as arrowing through mixed
/// history does. Each render's sync cost plus the dispatches that follow it.
fn language_switch_section(state: &Rc<AppState>) {
    println!("\n== code previews switching language (sync + following dispatches)");
    let entries = all_summaries(state);
    let mut picks: Vec<(rsclip_core::syntax::CodeLanguage, ClipboardEntry)> = Vec::new();
    for e in &entries {
        if !matches!(e.data, EntryData::Text) {
            continue;
        }
        let Some(lang) =
            rsclip_core::syntax::detect_code_language(e.preview_text.as_deref().unwrap_or(""))
        else {
            continue;
        };
        if picks.iter().all(|(l, _)| *l != lang) {
            picks.push((lang, e.clone()));
        }
        if picks.len() == 6 {
            break;
        }
    }
    for round in 1..=3 {
        for (lang, e) in &picks {
            state::invalidate_preview_cache(state);
            let origin = Instant::now();
            let (_, sync) = time(|| components::preview::render_preview(state, e));
            let d = pump(state, Duration::from_millis(120), origin);
            let worst = d.iter().map(|d| ms(d.dur)).fold(sync, f64::max);
            println!(
                "round {round} {:<12} sync {sync:7.2} ms, worst dispatch {worst:7.2} ms",
                format!("{lang:?}")
            );
        }
    }
}
