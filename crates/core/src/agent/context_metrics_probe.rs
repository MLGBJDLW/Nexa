//! Bounded manual microbenchmark of the real ContextMetrics implementation.
//! All setup, tokenizer initialization, barriers and checks stay outside the
//! reported phase timings. These are elapsed wall samples, never CPU/RSS data.

use std::hint::black_box;
use std::sync::Barrier;
use std::time::Instant;

use base64::Engine;
use serde::Serialize;

use super::*;
use crate::llm::prompt_cache::{resolve_prompt_cache_profile, PromptCacheApiStyle};
use crate::llm::provider_turn::RouteSnapshot;
use crate::llm::reasoning_profile::ReasoningReplayPolicy;
use crate::llm::{ContentPart, ProviderType};

const MODEL: &str = "gpt-4.1";
const MESSAGE_COUNT: usize = 128;
const WAVES: usize = 5;
const IMAGE_COUNT: usize = 12;
const TEXT_UNIT: &str = "source evidence alpha beta gamma delta epsilon;\n";

struct Fixture {
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    actual_estimated_tokens: u32,
    text_bytes: usize,
    image_bytes: usize,
    image_count: usize,
}

#[derive(Serialize)]
struct Quantiles {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

fn quantiles(values: impl IntoIterator<Item = f64>) -> Quantiles {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    assert!(!values.is_empty());
    let nearest_rank = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    Quantiles {
        samples: values.len(),
        p50_ms: nearest_rank(50),
        p95_ms: nearest_rank(95),
        max_ms: *values.last().unwrap(),
    }
}

#[derive(Serialize)]
struct WorkerSample {
    wave: usize,
    worker: usize,
    cold_ms: f64,
    warm_ms: f64,
    append_ms: f64,
    cold_message_analyses: usize,
    warm_new_message_analyses: usize,
    append_new_message_analyses: usize,
    tool_surface_analyses: usize,
    message_payloads_shared: bool,
    analysis_entries_shared: bool,
    analysis_entries_released_after_drop: bool,
    cached_serialized_bytes: usize,
    actual_estimated_tokens: u32,
    appended_estimated_tokens: u32,
}

#[derive(Serialize)]
struct MatrixRow {
    target_tokens: u32,
    actual_estimated_tokens: u32,
    message_count: usize,
    text_bytes: usize,
    image_count: usize,
    image_bytes: usize,
    workers: usize,
    waves: usize,
    read_only_message_views_per_worker: usize,
    case_total_wall_ms: f64,
    cold_per_worker: Quantiles,
    warm_per_worker: Quantiles,
    append_per_worker: Quantiles,
    cold_wave_max: Quantiles,
    warm_wave_max: Quantiles,
    append_wave_max: Quantiles,
    worker_samples: Vec<WorkerSample>,
}

fn valid_image_payload() -> String {
    // A deterministic noisy PNG avoids depending on external image files.
    // Encoding is setup only; the measured cache reads bytes, not image pixels.
    let pixels = image::RgbaImage::from_fn(512, 512, |x, y| {
        let mut value = (y * 512 + x).wrapping_add(1);
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        image::Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(encoded.into_inner())
}

fn fixture(target_tokens: u32, image_payload: Option<&str>) -> Fixture {
    let image_count = if image_payload.is_some() {
        IMAGE_COUNT
    } else {
        0
    };
    let image_bytes = image_payload.map_or(0, |payload| payload.len() * image_count);
    let image_tokens = image_payload.map_or(0, |payload| {
        ((payload.len() / 1500) as u32).max(258) * image_count as u32
    });
    let unit_tokens = estimate_tokens_for_model(MODEL, &TEXT_UNIT.repeat(256)) / 256;
    let repeats =
        target_tokens.saturating_sub(image_tokens) / MESSAGE_COUNT as u32 / unit_tokens.max(1);
    let mut messages = Vec::with_capacity(MESSAGE_COUNT);
    for index in 0..MESSAGE_COUNT {
        let body = format!("EVIDENCE_{index}:\n{}", TEXT_UNIT.repeat(repeats as usize));
        let mut message = if index < image_count {
            Message::text(Role::User, body)
        } else {
            Message::text_with_name(Role::Tool, body, format!("call-{index}"))
        };
        if index < image_count {
            message.parts.push(ContentPart::Image {
                media_type: "image/png".into(),
                data: image_payload.unwrap().to_owned(),
            });
        }
        messages.push(message);
    }
    let tools = vec![ToolDefinition {
        name: "read_fixture".into(),
        description: "Read the next evidence batch".into(),
        parameters: serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
    }];
    let actual_estimated_tokens =
        context::estimate_context_usage_breakdown_for_model(MODEL, &messages, &tools, None)
            .total_tokens;
    assert!(
        actual_estimated_tokens.abs_diff(target_tokens) <= target_tokens / 20 + 128,
        "fixture drifted from its target: {actual_estimated_tokens} vs {target_tokens}"
    );
    let text_bytes = messages
        .iter()
        .flat_map(|message| &message.parts)
        .map(|part| match part {
            ContentPart::Text { text } => text.len(),
            _ => 0,
        })
        .sum();
    Fixture {
        messages,
        tools,
        actual_estimated_tokens,
        text_bytes,
        image_bytes,
        image_count,
    }
}

fn shared_payloads(source: &[Message], view: &[Message]) -> bool {
    source.len() == view.len()
        && source.iter().zip(view).all(|(source, view)| {
            source.revision() == view.revision()
                && source.parts.len() == view.parts.len()
                && source
                    .parts
                    .iter()
                    .zip(&view.parts)
                    .all(|(left, right)| match (left, right) {
                        (ContentPart::Text { text: left }, ContentPart::Text { text: right }) => {
                            left.len() == right.len() && left.as_ptr() == right.as_ptr()
                        }
                        (
                            ContentPart::Image { data: left, .. },
                            ContentPart::Image { data: right, .. },
                        ) => left.len() == right.len() && left.as_ptr() == right.as_ptr(),
                        _ => false,
                    })
        })
}

fn run_worker(worker: usize, fixture: &Fixture, barrier: &Barrier) -> Vec<WorkerSample> {
    // Warm any tokenizer state local to this worker as well as the process-wide
    // tokenizer warmed by the parent. "Cold" means the ContextMetrics cache.
    black_box(estimate_tokens_for_model(MODEL, TEXT_UNIT));
    let route = RouteSnapshot::unknown("fixture", MODEL, ReasoningReplayPolicy::NotRequired);
    let profile_key = resolve_prompt_cache_profile(
        ProviderType::OpenAi,
        None,
        PromptCacheApiStyle::OpenAiCompatible,
        MODEL,
    )
    .key;
    let mut samples = Vec::with_capacity(WAVES);
    for wave in 0..WAVES {
        let mut request = fixture.messages.clone();
        let retry = request.clone();
        let replay = crate::llm::reasoning_replay::prepare_provider_replay_history(&retry, &route);
        let message_payloads_shared = shared_payloads(&fixture.messages, &request)
            && shared_payloads(&fixture.messages, &retry)
            && shared_payloads(&fixture.messages, &replay.messages);
        let mut metrics = ContextMetrics::default();
        barrier.wait();
        let start = Instant::now();
        let cold =
            black_box(metrics.snapshot(profile_key.clone(), MODEL, &request, &fixture.tools));
        let cold_ms = start.elapsed().as_secs_f64() * 1_000.0;
        let cold_message_analyses = metrics.analyzed_messages;
        barrier.wait();
        let start = Instant::now();
        let warm =
            black_box(metrics.snapshot(profile_key.clone(), MODEL, &request, &fixture.tools));
        let warm_ms = start.elapsed().as_secs_f64() * 1_000.0;
        let after_warm = metrics.analyzed_messages;
        request.push(Message::text_with_name(
            Role::Tool,
            TEXT_UNIT.repeat(32),
            "appended-call",
        ));
        barrier.wait();
        let start = Instant::now();
        let appended =
            black_box(metrics.snapshot(profile_key.clone(), MODEL, &request, &fixture.tools));
        let append_ms = start.elapsed().as_secs_f64() * 1_000.0;
        let after_append = metrics.analyzed_messages;
        barrier.wait();
        let analysis_entries_shared = cold
            .messages
            .iter()
            .zip(&warm.messages)
            .all(|(left, right)| Arc::ptr_eq(left, right))
            && cold
                .messages
                .iter()
                .zip(&appended.messages)
                .all(|(left, right)| Arc::ptr_eq(left, right));
        let weak_entries = appended
            .messages
            .iter()
            .map(Arc::downgrade)
            .collect::<Vec<_>>();
        let actual_estimated_tokens = cold.breakdown(None).total_tokens;
        let appended_estimated_tokens = appended.breakdown(None).total_tokens;
        let cached_serialized_bytes = appended
            .messages
            .iter()
            .map(|entry| entry.serialized.len())
            .sum();
        let tool_surface_analyses = metrics.analyzed_tool_surfaces;
        // All Message views are still alive while the pointer checks run.
        // Weak checks establish release of analysis entries, not OS RSS.
        drop((cold, warm, appended, metrics));
        let analysis_entries_released_after_drop =
            weak_entries.iter().all(|entry| entry.upgrade().is_none());
        drop((request, retry, replay));
        samples.push(WorkerSample {
            wave,
            worker,
            cold_ms,
            warm_ms,
            append_ms,
            cold_message_analyses,
            warm_new_message_analyses: after_warm - cold_message_analyses,
            append_new_message_analyses: after_append - after_warm,
            tool_surface_analyses,
            message_payloads_shared,
            analysis_entries_shared,
            analysis_entries_released_after_drop,
            cached_serialized_bytes,
            actual_estimated_tokens,
            appended_estimated_tokens,
        });
    }
    samples
}

#[test]
#[ignore = "bounded manual scale/concurrency matrix; prints wall-time evidence, not CPU or RSS"]
fn context_metrics_scale_and_concurrency_matrix() {
    // Warm the tokenizer and build valid image input before timing any cache.
    black_box(estimate_tokens_for_model(MODEL, TEXT_UNIT));
    let image = valid_image_payload();
    println!(
        "CONTEXT_METRICS_MATRIX_META {}",
        serde_json::json!({
            "fixtureVersion": 1, "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "availableParallelism": std::thread::available_parallelism().map_or(0, |value| value.get()),
            "modelForTokenizer": MODEL, "wavesPerCase": WAVES,
            "quantileMethod": "nearest-rank; n=5 p95 is max; n=20 pools 5 correlated four-worker waves",
            "coldMeaning": "fresh ContextMetrics only; tokenizer and process already initialized",
            "timingMeaning": "elapsed wall time per snapshot; excludes setup, barriers, checks, drop and wire serialization",
        "caseTotalMeaning": "includes worker thread startup, per-wave setup, barriers, checks and drop",
        "waveMaxMeaning": "maximum within-worker phase elapsed in each wave; not barrier-release-to-completion makespan",
        "imageBytesMeaning": "UTF-8/base64 ContentPart.data bytes; not decoded pixels or PNG binary size",
            "limitations": "synthetic repeated text; 12 equal valid PNG payloads; no image decoding, provider request, CPU profile, allocator/RSS, or complete AgentExecutor latency"
        })
    );
    for target_tokens in [32_768, 131_072, 524_288, 1_000_000] {
        for image_payload in [None, Some(image.as_str())] {
            let fixture = fixture(target_tokens, image_payload);
            for workers in [1, 4] {
                let barrier = Barrier::new(workers);
                let started = Instant::now();
                let worker_samples = std::thread::scope(|scope| {
                    let handles = (0..workers)
                        .map(|worker| {
                            let fixture = &fixture;
                            let barrier = &barrier;
                            scope.spawn(move || run_worker(worker, fixture, barrier))
                        })
                        .collect::<Vec<_>>();
                    handles
                        .into_iter()
                        .flat_map(|handle| handle.join().unwrap())
                        .collect::<Vec<_>>()
                });
                let case_total_wall_ms = started.elapsed().as_secs_f64() * 1_000.0;
                // Assertions run after every worker has left all barriers.
                for sample in &worker_samples {
                    assert_eq!(sample.cold_message_analyses, MESSAGE_COUNT);
                    assert_eq!(sample.warm_new_message_analyses, 0);
                    assert_eq!(sample.append_new_message_analyses, 1);
                    assert_eq!(sample.tool_surface_analyses, 1);
                    assert_eq!(
                        sample.actual_estimated_tokens,
                        fixture.actual_estimated_tokens
                    );
                    assert!(
                        sample.message_payloads_shared
                            && sample.analysis_entries_shared
                            && sample.analysis_entries_released_after_drop
                    );
                }
                let wave_max = |phase: fn(&WorkerSample) -> f64| {
                    quantiles((0..WAVES).map(|wave| {
                        worker_samples
                            .iter()
                            .filter(|sample| sample.wave == wave)
                            .map(phase)
                            .fold(0.0, f64::max)
                    }))
                };
                let row = MatrixRow {
                    target_tokens,
                    actual_estimated_tokens: fixture.actual_estimated_tokens,
                    message_count: MESSAGE_COUNT,
                    text_bytes: fixture.text_bytes,
                    image_count: fixture.image_count,
                    image_bytes: fixture.image_bytes,
                    workers,
                    waves: WAVES,
                    read_only_message_views_per_worker: 3,
                    case_total_wall_ms,
                    cold_per_worker: quantiles(worker_samples.iter().map(|sample| sample.cold_ms)),
                    warm_per_worker: quantiles(worker_samples.iter().map(|sample| sample.warm_ms)),
                    append_per_worker: quantiles(
                        worker_samples.iter().map(|sample| sample.append_ms),
                    ),
                    cold_wave_max: wave_max(|sample| sample.cold_ms),
                    warm_wave_max: wave_max(|sample| sample.warm_ms),
                    append_wave_max: wave_max(|sample| sample.append_ms),
                    worker_samples,
                };
                println!(
                    "CONTEXT_METRICS_MATRIX_ROW {}",
                    serde_json::to_string(&row).unwrap()
                );
            }
        }
    }
}
