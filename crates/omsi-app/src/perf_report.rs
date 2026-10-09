//! OMSI_PROFILE's exit summary (`--exit-after`) measured from the end of the warm-up (15 s
//! of play after the map has loaded, where the CPU mark is taken): frame-time percentiles
//! and the frames over 16.7/33.3/50/100 ms in the log, and with OMSI_PROFILE_JSON=<file>
//! the same summary with the stages and the GPU passes as a file, for
//! `scripts/compare-performance.py` to compare two runs of the same scene. Nothing of it
//! runs without OMSI_PROFILE: the frame times are kept only while profiling.

use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The frame times kept at most (about two hours at 144 fps).
pub(crate) const MAX_FRAMES: usize = 1 << 20;

/// The profile when the warm-up ended, for what the stages took after it.
pub(crate) struct ProfileMark {
    app: BTreeMap<&'static str, f64>,
    render: BTreeMap<&'static str, f64>,
    /// Per GPU pass: the milliseconds measured in all and the frames measured.
    gpu: BTreeMap<String, (f64, u32)>,
}

impl ProfileMark {
    pub(crate) fn take(app: &BTreeMap<&'static str, f64>, r: &omsi_render::Renderer) -> Self {
        ProfileMark {
            app: app.clone(),
            render: r.stats.borrow().clone(),
            gpu: r.gpu_pass_times().into_iter().map(|(k, ms, n)| (k, (ms * n as f64, n))).collect(),
        }
    }
}

/// Nearest rank: the smallest time that `p` of the frames do not exceed.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((p * sorted.len() as f64).ceil() as usize).saturating_sub(1).min(sorted.len() - 1)]
}

/// The frames after the warm-up: their number, time, rate, percentiles and slow frames.
pub(crate) fn frame_summary(frame_times: &[f32]) -> Value {
    let mut ms: Vec<f64> = frame_times.iter().map(|&t| t as f64 * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let seconds = ms.iter().sum::<f64>() / 1000.0;
    let over = |limit: f64| ms.iter().filter(|&&t| t > limit).count();
    json!({
        "frames": ms.len(), "measured_seconds": seconds,
        "average_fps": if seconds > 0.0 { ms.len() as f64 / seconds } else { 0.0 },
        "p50_ms": percentile(&ms, 0.5), "p95_ms": percentile(&ms, 0.95), "p99_ms": percentile(&ms, 0.99),
        "worst_frame_ms": ms.last().copied().unwrap_or(0.0),
        "frames_over_16_7_ms": over(16.7), "frames_over_33_3_ms": over(33.3),
        "frames_over_50_ms": over(50.0), "frames_over_100_ms": over(100.0),
        "frame_limit_reached": frame_times.len() >= MAX_FRAMES,
    })
}

/// The log line of [`frame_summary`].
pub(crate) fn log_line(s: &Value) -> String {
    let f = |k: &str| s[k].as_f64().unwrap_or(0.0);
    format!(
        "profile: since 15 s {} frames, {:.1} fps, frame time p50 {:.1} / p95 {:.1} / p99 {:.1} / worst {:.1} ms, frames over 16.7 / 33.3 / 50 / 100 ms: {} / {} / {} / {}",
        f("frames"), f("average_fps"), f("p50_ms"), f("p95_ms"), f("p99_ms"), f("worst_frame_ms"),
        f("frames_over_16_7_ms"), f("frames_over_33_3_ms"), f("frames_over_50_ms"), f("frames_over_100_ms")
    )
}

/// What each stage took per frame after the warm-up (ms; inclusive: a stage's sub-stages,
/// `a.b`, are part of `a`), and each GPU pass per measured frame.
fn stages_since(perf: &crate::app::PerfState, r: &omsi_render::Renderer, frames: usize) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let n = frames.max(1) as f64;
    let mark = perf.profile_mark.as_ref();
    let mut stages = BTreeMap::new();
    for (k, v) in &perf.profile {
        let before = mark.and_then(|m| m.app.get(k)).copied().unwrap_or(0.0);
        stages.insert(k.to_string(), (v - before).max(0.0) / n * 1000.0);
    }
    for (k, v) in r.stats.borrow().iter() {
        let before = mark.and_then(|m| m.render.get(k)).copied().unwrap_or(0.0);
        // (a renderer built again after a fallback counts from 0)
        let d = if *v >= before { v - before } else { *v };
        stages.insert(format!("render.{k}"), d / n * 1000.0);
    }
    let mut gpu = BTreeMap::new();
    for (pass, ms, count) in r.gpu_pass_times() {
        let (ms0, n0) = mark.and_then(|m| m.gpu.get(&pass)).copied().unwrap_or((0.0, 0));
        if let Some(k) = count.checked_sub(n0).filter(|k| *k > 0) {
            gpu.insert(pass, ((ms * count as f64 - ms0) / k as f64).max(0.0));
        }
    }
    (stages, gpu)
}

/// The summary as OMSI_PROFILE_JSON writes it: the run (to tell whether two runs can be
/// compared) and what was measured after the warm-up.
pub(crate) struct Run<'a> {
    pub perf: &'a crate::app::PerfState,
    pub settings: &'a crate::settings::Settings,
    pub args: &'a crate::cli::Args,
    pub lan: bool,
}

pub(crate) fn report(run: Run, r: &omsi_render::Renderer, window: (u32, u32), scene: (u32, u32), summary: Value) -> Value {
    let frames = summary["frames"].as_u64().unwrap_or(0) as usize;
    let (stages, gpu) = stages_since(run.perf, r, frames);
    let (s, perf, args) = (run.settings, run.perf, run.args);
    let mut summary = summary;
    summary["stage_average_ms"] = json!(stages);
    summary["gpu_pass_ms"] = json!(gpu);
    if let (Some((c0, t0, f0)), Some(c1)) = (perf.cpu_mark, crate::startup::process_cpu_seconds()) {
        let frames = perf.total_frames.saturating_sub(f0).max(1) as f64;
        summary["cpu_ms_per_frame"] = json!((c1 - c0) / frames * 1000.0);
        summary["cores_busy"] = json!((c1 - c0) / t0.elapsed().as_secs_f64().max(1e-3));
        if let (Some(m0), Some(m1)) = (perf.thread_cpu_mark, crate::startup::thread_cpu_seconds()) {
            summary["main_thread_cpu_ms_per_frame"] = json!((m1 - m0) / frames * 1000.0);
        }
        if let (Some(i0), Some(i1)) = (perf.instructions_mark, crate::startup::process_instructions()) {
            summary["million_instructions_per_frame"] = json!(i1.saturating_sub(i0) as f64 / frames / 1e6);
        }
    }
    json!({
        "schema_version": 1,
        "version": crate::startup::VERSION, "build": crate::startup::BUILD,
        "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
        "adapter": r.adapter_name,
        "warmup_seconds": 15,
        "run": {
            "map": args.map, "bus": args.bus, "seconds": args.exit_after,
            "window": [window.0, window.1], "scene": [scene.0, scene.1],
            "msaa": r.options.msaa, "ssao": r.options.ssao, "shadow_size": r.options.shadow_size,
            "graphics": s.graphics, "render_scale": s.render_scale, "shadows": s.shadows,
            "mirrors": s.mirror_refresh, "reflections": s.reflections, "max_fps": s.max_fps, "vsync": s.vsync,
            "lan": run.lan,
        },
        "summary": summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles_and_slow_frames() {
        let s = frame_summary(&[0.010, 0.020, 0.040, 0.060, 0.120]);
        let ms = |k: &str| s[k].as_f64().unwrap();
        assert!((ms("p50_ms") - 40.0).abs() < 1e-3);
        assert!((ms("p95_ms") - 120.0).abs() < 1e-3 && (ms("p99_ms") - 120.0).abs() < 1e-3);
        assert_eq!(
            ["frames_over_16_7_ms", "frames_over_33_3_ms", "frames_over_50_ms", "frames_over_100_ms"].map(|k| s[k].as_u64().unwrap()),
            [4, 3, 2, 1]
        );
        // (the frame rate is the frames over their time, not an average of rates)
        assert!((ms("average_fps") - 20.0).abs() < 1e-3);
        assert_eq!(frame_summary(&[])["average_fps"], 0.0);
    }
}
