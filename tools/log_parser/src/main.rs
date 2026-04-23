use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

const REQUIRED_MARKERS: &[(&str, &str)] = &[
    ("bootloader", "Prometheus Bootloader"),
    ("gop", "GOP OK"),
    ("disk", "Disk OK"),
    ("kernel_loaded", "Kernel loaded"),
    ("exit_bs", "ExitBootServices OK"),
    ("jump", "Jump to kernel"),
    ("k00", "K00 kernel entry"),
    ("k07", "K07 fabric done"),
    ("r00", "R00 runtime start"),
    ("r10", "R10 tick="),
    ("s10", "S10 cur="),
    ("t01", "T01 logic alive"),
    ("t02", "T02 alive"),
];

const RESET_HINTS: &[&str] = &["triple fault", "cpu reset", " reset "];

#[derive(Clone, Copy, Debug)]
enum Mode {
    Baseline,
    Diagnostic,
    ExcAcceptance,
}

#[derive(Serialize)]
struct Markers {
    required: BTreeMap<String, bool>,
    required_missing: Vec<String>,
    runtime_started: bool,
    liveness_ok: bool,
    tick_monotonic: bool,
}

#[derive(Serialize)]
struct Metrics {
    tick_samples: usize,
    tick_first: Option<u64>,
    tick_last: Option<u64>,
    hard_error_total: u64,
}

#[derive(Serialize)]
struct ResetSignals {
    boot_repeat_after_runtime: bool,
    explicit_reset_hints: u64,
}

#[derive(Serialize)]
struct Report {
    log_path: String,
    mode: String,
    pass: bool,
    verdict: String,
    markers: Markers,
    errors: BTreeMap<String, u64>,
    metrics: Metrics,
    reset_or_silent_fault: bool,
    reset_signals: ResetSignals,
    notes: Vec<String>,
}

fn parse_args() -> Result<(String, String, Mode), String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut mode = Mode::Baseline;

    let args: Vec<String> = env::args().collect();
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --input".to_string());
                }
                input = Some(args[i].clone());
            }
            "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --output".to_string());
                }
                output = Some(args[i].clone());
            }
            "--mode" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --mode".to_string());
                }
                mode = match args[i].as_str() {
                    "baseline" => Mode::Baseline,
                    "diagnostic" => Mode::Diagnostic,
                    "exc_acceptance" => Mode::ExcAcceptance,
                    _ => return Err("--mode must be baseline|diagnostic|exc_acceptance".to_string()),
                };
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            x => return Err(format!("unknown argument: {x}")),
        }
        i += 1;
    }

    match (input, output) {
        (Some(inp), Some(out)) => Ok((inp, out, mode)),
        _ => Err("usage: --input <raw.log> --output <report.json> [--mode baseline|diagnostic|exc_acceptance]"
            .to_string()),
    }
}

fn print_help() {
    println!("prometheus-log-parser");
    println!("  --input <raw.log> --output <report.json> [--mode baseline|diagnostic|exc_acceptance]");
}

fn count_contains(text_lower: &str, needle: &str) -> u64 {
    text_lower.matches(needle).count() as u64
}

fn decode_text(raw_bytes: &[u8]) -> String {
    if raw_bytes.len() >= 2 {
        if raw_bytes[0] == 0xFF && raw_bytes[1] == 0xFE {
            let mut u16s = Vec::new();
            let mut i = 2usize;
            while i + 1 < raw_bytes.len() {
                u16s.push(u16::from_le_bytes([raw_bytes[i], raw_bytes[i + 1]]));
                i += 2;
            }
            return String::from_utf16_lossy(&u16s);
        }
        if raw_bytes[0] == 0xFE && raw_bytes[1] == 0xFF {
            let mut u16s = Vec::new();
            let mut i = 2usize;
            while i + 1 < raw_bytes.len() {
                u16s.push(u16::from_be_bytes([raw_bytes[i], raw_bytes[i + 1]]));
                i += 2;
            }
            return String::from_utf16_lossy(&u16s);
        }
    }

    let mut zero_odd = 0usize;
    let mut checked = 0usize;
    let sample = core::cmp::min(raw_bytes.len(), 4096);
    let mut i = 1usize;
    while i < sample {
        checked += 1;
        if raw_bytes[i] == 0 {
            zero_odd += 1;
        }
        i += 2;
    }

    if checked > 32 && (zero_odd * 100 / checked) >= 60 {
        let mut u16s = Vec::new();
        let mut j = 0usize;
        while j + 1 < raw_bytes.len() {
            u16s.push(u16::from_le_bytes([raw_bytes[j], raw_bytes[j + 1]]));
            j += 2;
        }
        return String::from_utf16_lossy(&u16s);
    }

    String::from_utf8_lossy(raw_bytes).into_owned()
}

fn extract_ticks(text: &str) -> Vec<u64> {
    let mut ticks = Vec::new();
    for line in text.lines() {
        if let Some(pos) = line.find("R10 tick=") {
            let rest = &line[pos + "R10 tick=".len()..];
            let mut val = String::new();
            for ch in rest.chars() {
                if ch.is_ascii_digit() {
                    val.push(ch);
                } else {
                    break;
                }
            }
            if !val.is_empty() {
                if let Ok(n) = val.parse::<u64>() {
                    ticks.push(n);
                }
            }
        }
    }
    ticks
}

fn is_tick_monotonic(ticks: &[u64]) -> bool {
    if ticks.is_empty() {
        return false;
    }
    let mut prev = ticks[0];
    for &cur in &ticks[1..] {
        if cur <= prev {
            return false;
        }
        prev = cur;
    }
    true
}

fn has_boot_repeat_after_runtime(text: &str) -> bool {
    let mut saw_runtime = false;
    for line in text.lines() {
        if line.contains("R00 runtime start") {
            saw_runtime = true;
            continue;
        }
        if saw_runtime && line.contains("Prometheus Bootloader") {
            return true;
        }
    }
    false
}


fn detect_exception_tag(text_lower: &str) -> Option<&'static str> {
    if text_lower.contains("xud") {
        Some("XUD")
    } else if text_lower.contains("xgp") {
        Some("XGP")
    } else if text_lower.contains("xpf") {
        Some("XPF")
    } else if text_lower.contains("xdf") {
        Some("XDF")
    } else {
        None
    }
}
fn analyze(log_path: &str, mode: Mode) -> Result<Report, String> {
    let raw_bytes = fs::read(log_path).map_err(|e| format!("failed to read input: {e}"))?;
    let raw = decode_text(&raw_bytes);
    let raw_lower = raw.to_lowercase();

    let mut required = BTreeMap::new();
    for (k, marker) in REQUIRED_MARKERS {
        required.insert((*k).to_string(), raw.contains(marker));
    }

    let mut required_missing = Vec::new();
    for (k, v) in &required {
        if !*v {
            required_missing.push(k.clone());
        }
    }

    let mut errors = BTreeMap::new();
    errors.insert("E20".to_string(), count_contains(&raw_lower, "e20 ovf"));
    errors.insert("E30".to_string(), count_contains(&raw_lower, "e30 evq_ovf"));
    errors.insert("E31".to_string(), count_contains(&raw_lower, "e31 starve"));
    errors.insert("E40".to_string(), count_contains(&raw_lower, "e40 task_ovf_seen"));
    errors.insert("E50".to_string(), count_contains(&raw_lower, "e50 dl_viol"));
    errors.insert("E51".to_string(), count_contains(&raw_lower, "e51 dl_sum"));
    errors.insert(
        "panic".to_string(),
        count_contains(&raw_lower, "p00 panic") + count_contains(&raw_lower, " panic"),
    );
    errors.insert(
        "exc".to_string(),
        count_contains(&raw_lower, "xud")
            + count_contains(&raw_lower, "xgp")
            + count_contains(&raw_lower, "xpf")
            + count_contains(&raw_lower, "xdf"),
    );

    let ticks = extract_ticks(&raw);
    let tick_monotonic = is_tick_monotonic(&ticks);

    let boot_repeat_after_runtime = has_boot_repeat_after_runtime(&raw);
    let mut explicit_reset_hints = 0u64;
    for hint in RESET_HINTS {
        explicit_reset_hints += count_contains(&raw_lower, hint);
    }

    let hard_error_total = errors.get("E20").unwrap_or(&0)
        + errors.get("E30").unwrap_or(&0)
        + errors.get("E31").unwrap_or(&0)
        + errors.get("E40").unwrap_or(&0)
        + errors.get("E50").unwrap_or(&0)
        + errors.get("E51").unwrap_or(&0)
        + errors.get("panic").unwrap_or(&0)
        + errors.get("exc").unwrap_or(&0);
    let mut notes = Vec::new();
    let required_ok = required_missing.is_empty();
    let reset_ok = !boot_repeat_after_runtime && explicit_reset_hints == 0;

    let pass = match mode {
        Mode::Baseline => {
            if !required_missing.is_empty() {
                notes.push(format!(
                    "missing_required_markers:{}",
                    required_missing.join(",")
                ));
            }
            if !tick_monotonic {
                notes.push("tick_not_monotonic_or_missing".to_string());
            }
            if boot_repeat_after_runtime {
                notes.push("boot_sequence_repeated_after_runtime".to_string());
            }
            if explicit_reset_hints > 0 {
                notes.push("reset_hints_detected".to_string());
            }
            if hard_error_total > 0 {
                notes.push(format!("hard_errors={}", hard_error_total));
            }
            required_ok && tick_monotonic && hard_error_total == 0 && reset_ok
        }
        Mode::Diagnostic => {
            if !required_missing.is_empty() {
                notes.push(format!(
                    "missing_required_markers:{}",
                    required_missing.join(",")
                ));
            }
            if !tick_monotonic {
                notes.push("tick_not_monotonic_or_missing".to_string());
            }
            if boot_repeat_after_runtime {
                notes.push("boot_sequence_repeated_after_runtime".to_string());
            }
            if explicit_reset_hints > 0 {
                notes.push("reset_hints_detected".to_string());
            }
            if hard_error_total > 0 {
                notes.push(format!("hard_errors={}", hard_error_total));
            }
            required_ok
                && tick_monotonic
                && *errors.get("panic").unwrap_or(&0) == 0
                && *errors.get("exc").unwrap_or(&0) == 0
                && reset_ok
        }
        Mode::ExcAcceptance => {
            let expected = detect_exception_tag(&raw_lower);
            if let Some(tag) = expected {
                notes.push(format!("exception={}", tag));
            } else {
                notes.push("exception=none".to_string());
            }

            if boot_repeat_after_runtime {
                notes.push("boot_sequence_repeated_after_runtime".to_string());
            }
            if explicit_reset_hints > 0 {
                notes.push("reset_hints_detected".to_string());
            }

            let pre_keys = [
                "bootloader",
                "gop",
                "disk",
                "kernel_loaded",
                "exit_bs",
                "jump",
                "k00",
                "k07",
                "r00",
            ];
            let mut missing_pre = Vec::new();
            for k in pre_keys {
                if !*required.get(k).unwrap_or(&false) {
                    missing_pre.push(k.to_string());
                }
            }
            if !missing_pre.is_empty() {
                notes.push(format!("missing_pre_exception:{}", missing_pre.join(",")));
            }

            false
        }
    };
    Ok(Report {
        log_path: log_path.to_string(),
        mode: match mode {
            Mode::Baseline => "baseline".to_string(),
            Mode::Diagnostic => "diagnostic".to_string(),
            Mode::ExcAcceptance => "exc_acceptance".to_string(),
        },
        pass,
        verdict: if pass { "PASS" } else { "FAIL" }.to_string(),
        markers: Markers {
            runtime_started: *required.get("r00").unwrap_or(&false),
            liveness_ok: *required.get("r10").unwrap_or(&false)
                && *required.get("s10").unwrap_or(&false)
                && *required.get("t01").unwrap_or(&false)
                && *required.get("t02").unwrap_or(&false),
            required,
            required_missing,
            tick_monotonic,
        },
        errors,
        metrics: Metrics {
            tick_samples: ticks.len(),
            tick_first: ticks.first().copied(),
            tick_last: ticks.last().copied(),
            hard_error_total,
        },
        reset_or_silent_fault: boot_repeat_after_runtime || explicit_reset_hints > 0,
        reset_signals: ResetSignals {
            boot_repeat_after_runtime,
            explicit_reset_hints,
        },
        notes,
    })
}

fn main() {
    let (input, output, mode) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("FAIL {e}");
            std::process::exit(2);
        }
    };

    if !Path::new(&input).exists() {
        eprintln!("FAIL input_not_found: {input}");
        std::process::exit(2);
    }

    let report = match analyze(&input, mode) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAIL {e}");
            std::process::exit(2);
        }
    };

    let output_path = Path::new(&output);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                eprintln!("FAIL cannot create output dir: {e}");
                std::process::exit(2);
            }
        }
    }

    let json = match serde_json::to_string_pretty(&report) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FAIL json_encode: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) = fs::write(output_path, json) {
        eprintln!("FAIL write_report: {e}");
        std::process::exit(2);
    }

    if report.pass {
        println!("PASS {}", input);
        std::process::exit(0);
    }

    let reason = if report.notes.is_empty() {
        "invariant_violation".to_string()
    } else {
        report.notes.join(";")
    };
    println!("FAIL {} :: {}", input, reason);
    std::process::exit(1);
}




