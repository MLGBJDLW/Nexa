#![cfg(feature = "video")]
use nexa_core::video::{extract_analysis_frames, extract_frames, extract_keyframes, VideoConfig};
use std::{path::Path, process::Command, time::Instant};
fn generate(ffmpeg: &str, output: &Path, inputs: &[&str], concat: bool) {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    for input in inputs {
        command.args(["-f", "lavfi", "-i", input]);
    }
    if concat {
        command.args([
            "-filter_complex",
            "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
            "-map",
            "[v]",
        ]);
    }
    command
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
#[ignore = "requires NEXA_TEST_FFMPEG pointing to a real FFmpeg executable"]
fn single_decode_keeps_opening_scene_changes_and_periodic_coverage() {
    let ffmpeg = std::env::var("NEXA_TEST_FFMPEG").expect("Set NEXA_TEST_FFMPEG");
    let directory = tempfile::tempdir().unwrap();
    let mut config = VideoConfig::default();
    config.ffmpeg_path = Some(ffmpeg.clone());
    config.frame_interval_secs = 2;
    config.scene_threshold = 0.3;
    let cut = directory.path().join("cuts.mp4");
    generate(
        &ffmpeg,
        &cut,
        &[
            "color=c=black:s=320x180:r=10:d=3",
            "color=c=white:s=320x180:r=10:d=3",
            "color=c=black:s=320x180:r=10:d=3",
        ],
        true,
    );
    let old = extract_keyframes(&cut, &directory.path().join("old-cuts"), 0.3, &config).unwrap();
    let frames =
        extract_analysis_frames(&cut, &directory.path().join("new-cuts"), &config).unwrap();
    let times = frames
        .iter()
        .map(|frame| frame.timestamp_ms)
        .collect::<Vec<_>>();
    assert_eq!(times.first(), Some(&0));
    assert!(times.contains(&3000));
    assert!(times.contains(&6000));
    assert!(times.windows(2).all(|pair| pair[1] - pair[0] <= 2000));
    assert!(times.last().copied().unwrap() >= 8000);
    assert!(old.len() < frames.len());
    let still = directory.path().join("still.mp4");
    generate(
        &ffmpeg,
        &still,
        &["color=c=black:s=1280x720:r=30:d=60"],
        false,
    );
    let started = Instant::now();
    let empty =
        extract_keyframes(&still, &directory.path().join("old-scenes"), 0.3, &config).unwrap();
    assert!(empty.is_empty());
    let old_frames =
        extract_frames(&still, &directory.path().join("old-periodic"), 2, &config).unwrap();
    let old_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let new_frames =
        extract_analysis_frames(&still, &directory.path().join("new-periodic"), &config).unwrap();
    let new_ms = started.elapsed().as_millis();
    assert_eq!(old_frames.len(), new_frames.len());
    assert_eq!(new_frames[0].timestamp_ms, 0);
    println!(
        "video-frame-evidence {}",
        serde_json::json!({"staticDurationSeconds":60,"legacyDecodes":2,"currentDecodes":1,"legacyMs":old_ms,"currentMs":new_ms,"staticFrames":new_frames.len(),"cutTimestampsMs":times})
    );
}
