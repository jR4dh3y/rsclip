use std::fmt;
use std::fs;
use std::io::IsTerminal;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseRecord {
    pub name: String,
    pub elapsed_ms: f64,
    pub memory_before_kb: u64,
    pub memory_after_kb: u64,
    /// Nesting depth; 0 = top-level phase, >0 = sub-phase.
    pub depth: usize,
    /// True for records that were skipped; shown as SKIPPED in the report.
    pub skipped: bool,
}

impl PhaseRecord {
    pub fn memory_delta_kb(&self) -> i64 {
        self.memory_after_kb as i64 - self.memory_before_kb as i64
    }
}

#[derive(Debug)]
struct PhaseFrame {
    name: String,
    start: Instant,
    memory_before_kb: u64,
}

/// A node in the phase tree, reconstructed from the flat records.
struct TreeNode {
    record: PhaseRecord,
    children: Vec<TreeNode>,
}

#[derive(Debug, Default)]
pub struct Profiler {
    pub enabled: bool,
    pub verbose: bool,
    pub records: Vec<PhaseRecord>,
    start_time: Option<Instant>,
    total_memory_before_kb: u64,
    stack: Vec<PhaseFrame>,
}

type ReportEntry = (String, f64, u64, u64, bool, bool);

impl Profiler {
    pub fn new(enabled: bool, verbose: bool) -> Self {
        Self {
            enabled,
            verbose,
            records: Vec::new(),
            start_time: None,
            total_memory_before_kb: 0,
            stack: Vec::new(),
        }
    }

    pub fn begin(&mut self) {
        if !self.enabled {
            return;
        }
        self.start_time = Some(Instant::now());
        self.total_memory_before_kb = read_rss_kb().unwrap_or(0);
    }

    pub fn begin_phase(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        if self.start_time.is_none() {
            self.begin();
        }
        self.stack.push(PhaseFrame {
            name: name.to_string(),
            start: Instant::now(),
            memory_before_kb: read_rss_kb().unwrap_or(0),
        });
    }

    pub fn end_phase(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        // Pop the frame for this phase; tolerate mismatched name (just pop top).
        let frame = match self.stack.iter().rposition(|f| f.name == name) {
            Some(idx) => self.stack.remove(idx),
            None => match self.stack.pop() {
                Some(f) => f,
                None => return,
            },
        };
        let elapsed_ms = frame.start.elapsed().as_secs_f64() * 1000.0;
        let memory_after_kb = read_rss_kb().unwrap_or(0);
        self.records.push(PhaseRecord {
            name: frame.name,
            elapsed_ms,
            memory_before_kb: frame.memory_before_kb,
            memory_after_kb,
            depth: self.stack.len(),
            skipped: false,
        });
    }

    /// Record an instantaneous, skipped phase. Rendered as SKIPPED rather than a timing.
    pub fn skip_phase(&mut self, name: &str) {
        if !self.enabled {
            return;
        }
        let memory_kb = read_rss_kb().unwrap_or(0);
        self.records.push(PhaseRecord {
            name: name.to_string(),
            elapsed_ms: 0.0,
            memory_before_kb: memory_kb,
            memory_after_kb: memory_kb,
            depth: self.stack.len(),
            skipped: true,
        });
    }

    pub fn reset(&mut self) {
        self.records.clear();
        self.stack.clear();
        self.start_time = None;
        self.total_memory_before_kb = 0;
    }

    fn render_report_line(
        &self,
        name: &str,
        width: usize,
        elapsed_ms: f64,
        before: u64,
        after: u64,
        skipped: bool,
    ) -> String {
        if skipped {
            return format!(
                "{name:<width$} {:<10} {:<10} {:<10} {:<10}",
                "SKIPPED", "-", "-", "-"
            );
        }
        let delta = after as i64 - before as i64;
        let delta_str = if delta >= 0 {
            format!("+{delta}")
        } else {
            format!("{delta}")
        };
        format!(
            "{name:<width$} {elapsed_ms:>10.2} {before:>10} KB {after:>10} KB {delta_str:>10} KB"
        )
    }

    /// Build the phase tree from flat records (which arrive children-first,
    /// each phase ending before its parent). A record's children are the
    /// records immediately preceding it whose depth is greater.
    fn build_tree(records: &[PhaseRecord]) -> Vec<TreeNode> {
        let mut nodes: Vec<TreeNode> = Vec::new();
        for record in records {
            let depth = record.depth;
            let mut child_start = nodes.len();
            while child_start > 0 && nodes[child_start - 1].record.depth > depth {
                child_start -= 1;
            }
            let children: Vec<TreeNode> = nodes.drain(child_start..).collect();
            nodes.push(TreeNode {
                record: record.clone(),
                children,
            });
        }
        nodes
    }

    /// Collect (tree-prefixed name, timings, is_sub, skipped) for every record.
    fn collect_tree(
        &self,
        nodes: &[TreeNode],
        prefix: &str,
        root_level: bool,
        out: &mut Vec<(String, f64, u64, u64, bool, bool)>,
    ) {
        for (i, node) in nodes.iter().enumerate() {
            let is_last = i + 1 == nodes.len();
            let connector = if root_level {
                String::new()
            } else if is_last {
                "└── ".to_string()
            } else {
                "├── ".to_string()
            };
            let child_prefix = if root_level {
                String::new()
            } else if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            let name = format!("{prefix}{connector}{}", node.record.name);
            out.push((
                name,
                node.record.elapsed_ms,
                node.record.memory_before_kb,
                node.record.memory_after_kb,
                !root_level,
                node.record.skipped,
            ));
            self.collect_tree(&node.children, &child_prefix, false, out);
        }
    }

    /// Report entries plus the fixed width of the name column, so the time and
    /// memory columns align regardless of tree depth or name length.
    fn report_entries(&self) -> (Vec<ReportEntry>, usize) {
        let mut entries = Vec::new();
        self.collect_tree(&Self::build_tree(&self.records), "", true, &mut entries);
        let width = entries
            .iter()
            .map(|(name, _, _, _, _, _)| name.len())
            .max()
            .unwrap_or(24)
            .max(24);
        (entries, width)
    }

    fn report_lines(&self) -> (Vec<(String, bool)>, usize) {
        let (entries, width) = self.report_entries();
        let lines = entries
            .into_iter()
            .map(|(name, elapsed_ms, before, after, sub, skipped)| {
                (
                    self.render_report_line(&name, width, elapsed_ms, before, after, skipped),
                    sub,
                )
            })
            .collect();
        (lines, width)
    }

    pub fn print_report(&self) {
        if !self.enabled || self.records.is_empty() {
            return;
        }

        let total_time_ms: f64 = self
            .records
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.elapsed_ms)
            .sum();
        let total_mem_delta_kb: i64 = self
            .records
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.memory_delta_kb())
            .sum();
        let peak_mem_kb = self
            .records
            .iter()
            .map(|r| r.memory_after_kb)
            .max()
            .unwrap_or(0);

        let is_tty = std::io::stderr().is_terminal();
        let cyan = if is_tty { "\x1b[1;96m" } else { "" };
        let dim = if is_tty { "\x1b[90m" } else { "" };
        let reset = if is_tty { "\x1b[0m" } else { "" };

        eprintln!("\n{cyan}=== rsclip Profile Report ==={reset}");
        let (lines, width) = self.report_lines();
        let rule = "─".repeat(width + 1 + 10 + 1 + 13 + 1 + 13 + 1 + 13);
        eprintln!(
            "{:<width$} {:>10} {:>13} {:>13} {:>13}",
            "Phase", "Time (ms)", "Mem Before", "Mem After", "Delta (KB)"
        );
        eprintln!("{dim}{rule}{reset}");

        for (line, sub) in lines {
            if sub {
                eprintln!("{dim}{line}{reset}");
            } else {
                eprintln!("{line}");
            }
        }

        eprintln!("{dim}{rule}{reset}");
        eprintln!(
            "{:<width$} {:>10.2} {:>34} {:>10} KB",
            "Total", total_time_ms, "", peak_mem_kb,
        );
        eprintln!(
            "{:<width$} {:>36} {:>10} KB",
            "",
            "",
            format!("net: {total_mem_delta_kb:+}"),
        );
        eprintln!();
    }
}

static CURRENT: Mutex<Option<Profiler>> = Mutex::new(None);

/// Install the process-wide profiler (replaces any previous one).
pub fn install(profiler: Profiler) {
    if let Ok(mut guard) = CURRENT.lock() {
        *guard = Some(profiler);
    }
}

/// Run `f` with the installed profiler; auto-initializes from `RSCLIP_PROFILE` env var if not yet installed.
fn with_profiler<R>(f: impl FnOnce(&mut Profiler) -> R) -> R {
    let mut guard = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let profiler = guard.get_or_insert_with(|| {
        if let Ok(val) = std::env::var("RSCLIP_PROFILE") {
            let verbose = val == "verbose" || val == "2";
            Profiler::new(true, verbose)
        } else {
            Profiler::new(false, false)
        }
    });
    f(profiler)
}

/// Returns true if verbose profiling output is requested.
pub fn verbose() -> bool {
    with_profiler(|p| p.verbose)
}

/// Returns true if profiling is enabled either explicitly or via RSCLIP_PROFILE.
pub fn enabled() -> bool {
    with_profiler(|p| p.enabled)
}

/// Starts the global profiler session.
pub fn begin() {
    with_profiler(|p| p.begin());
}

/// Begins a named profiling phase.
pub fn begin_phase(name: &str) {
    with_profiler(|p| p.begin_phase(name));
}

/// Ends a named profiling phase and records its elapsed time and memory delta.
pub fn end_phase(name: &str) {
    with_profiler(|p| p.end_phase(name));
}

/// Records a named phase as skipped in the profile report.
pub fn skip_phase(name: &str) {
    with_profiler(|p| p.skip_phase(name));
}

/// Formats and prints the hierarchical profile report to stderr.
pub fn print_report() {
    with_profiler(|p| p.print_report());
}

/// Resets the global profiler, clearing all recorded phases.
pub fn reset() {
    with_profiler(|p| p.reset());
}

/// Reads the current process VmRSS in kilobytes from `/proc/self/status`.
pub fn read_rss_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Formats a memory size in kilobytes to human-readable KB, MB, or GB.
pub fn format_bytes(kb: u64) -> String {
    if kb < 1024 {
        format!("{kb} KB")
    } else if kb < 1024 * 1024 {
        format!("{:.1} MB", kb as f64 / 1024.0)
    } else {
        format!("{:.2} GB", kb as f64 / (1024.0 * 1024.0))
    }
}

impl fmt::Display for Profiler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.enabled || self.records.is_empty() {
            return Ok(());
        }

        let total_time_ms: f64 = self
            .records
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.elapsed_ms)
            .sum();
        let total_mem_delta_kb: i64 = self
            .records
            .iter()
            .filter(|r| r.depth == 0)
            .map(|r| r.memory_delta_kb())
            .sum();
        let peak_mem_kb = self
            .records
            .iter()
            .map(|r| r.memory_after_kb)
            .max()
            .unwrap_or(0);

        writeln!(f, "\n=== rsclip Profile Report ===")?;
        let (lines, width) = self.report_lines();
        let rule = "─".repeat(width + 1 + 10 + 1 + 13 + 1 + 13 + 1 + 13);
        writeln!(
            f,
            "{:<width$} {:>10} {:>13} {:>13} {:>13}",
            "Phase", "Time (ms)", "Mem Before", "Mem After", "Delta (KB)"
        )?;
        writeln!(f, "{rule}")?;

        for (line, _sub) in lines {
            writeln!(f, "{line}")?;
        }

        writeln!(f, "{rule}")?;
        writeln!(
            f,
            "{:<width$} {:>10.2} {:>34} {:>10} KB",
            "Total", total_time_ms, "", peak_mem_kb,
        )?;
        writeln!(
            f,
            "{:<width$} {:>36} {:>10} KB",
            "",
            "",
            format!("net: {total_mem_delta_kb:+}"),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, depth: usize) -> PhaseRecord {
        PhaseRecord {
            name: name.to_string(),
            elapsed_ms: 1.0,
            memory_before_kb: 0,
            memory_after_kb: 0,
            depth,
            skipped: false,
        }
    }

    #[test]
    fn builds_tree_from_children_first_records() {
        // Flat records arrive in completion order (children finish before parent).
        let records = vec![
            rec("read list cache", 0),
            rec("query db row", 2),
            rec("read image thumb", 2),
            rec("decode thumbnail", 2),
            rec("query ocr text", 3),
            rec("parse ocr layout", 3),
            rec("format ocr preview", 2),
            rec("build preview panel", 1),
            rec("render clipboard preview", 0),
            rec("update footer", 0),
        ];
        let tree = Profiler::build_tree(&records);
        fn names(nodes: &[TreeNode]) -> Vec<&str> {
            nodes.iter().map(|n| n.record.name.as_str()).collect()
        }
        assert_eq!(
            names(&tree),
            vec![
                "read list cache",
                "render clipboard preview",
                "update footer"
            ]
        );
        let preview = &tree[1];
        assert_eq!(names(&preview.children), vec!["build preview panel"]);
        let panel = &preview.children[0];
        assert_eq!(
            names(&panel.children),
            vec![
                "query db row",
                "read image thumb",
                "decode thumbnail",
                "format ocr preview"
            ]
        );
        let ocr = &panel.children[3];
        assert_eq!(
            names(&ocr.children),
            vec!["query ocr text", "parse ocr layout"]
        );
    }

    #[test]
    fn profiler_phases_and_display() {
        let mut profiler = Profiler::new(true, true);
        profiler.begin();
        profiler.begin_phase("total operation");
        profiler.begin_phase("sub operation");
        profiler.end_phase("sub operation");
        profiler.skip_phase("skipped operation");
        profiler.end_phase("total operation");

        assert_eq!(profiler.records.len(), 3);
        let display = format!("{profiler}");
        assert!(display.contains("=== rsclip Profile Report ==="));
        assert!(display.contains("total operation"));
        assert!(display.contains("sub operation"));
        assert!(display.contains("SKIPPED"));
    }
}
