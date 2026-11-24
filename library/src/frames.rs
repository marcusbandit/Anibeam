//! A frame of the episode's own video for an episode with no still (#1469).
//! Continue shows the still of the episode to go on with, and many episodes
//! have none: ani.zip and TVmaze gave nothing. The stand-in is a frame of
//! interest from the file itself, never the show's banner or poster.
//!
//! A frame is taken for the episode a series' Continue card would show (the
//! one stopped short, else the next one up) once the series is started, when
//! a read of the series finds it with no still and no frame cached. No timer
//! and no walk of the library: the reads Home makes at launch and after every
//! change of progress are what ask. ffprobe reads the runtime, eight frames
//! between 30 and 70 percent of it (clear of an opening and an ending) are
//! decoded small and scored by [`interest`], and the best is written once as
//! a JPEG under `<cache>/frames/`, named for the file's path, size and mtime,
//! so a replaced file gets a new one. A video no frame comes of leaves an
//! empty `.miss` beside that name instead, so it is not tried again on the
//! next launch. One ffmpeg runs at a time, at the lowest
//! priority, and a machine without ffmpeg says so once in the trace and
//! takes no frames.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anibeam_contract::*;
use anibeam_foundation::process::command;
use anibeam_foundation::services::Services;
use anibeam_foundation::sync::recover;
use tokio::process::Command;

use crate::cards;

/// Frames scored per episode.
const SAMPLES: usize = 8;
/// The share of the runtime the samples spread over, clear of an opening and
/// an ending.
const SPREAD: (f64, f64) = (0.30, 0.70);
/// The width a sample is decoded at for scoring.
const SAMPLE_WIDTH: usize = 160;
/// The widest frame written, never wider than the video itself.
const FRAME_WIDTH: u32 = 1280;
/// The longest frames that landed wait to be announced while the queue is
/// still going.
const ANNOUNCE_EVERY: Duration = Duration::from_secs(20);
/// How long one ffprobe or ffmpeg run may take.
const RUN_LIMIT: Duration = Duration::from_secs(30);
/// Mean luma outside this is a black or a white frame.
const LUMA_RANGE: (f64, f64) = (35.0, 235.0);
/// A luma spread under this is a flat frame: a fade, a card, a wall.
const MIN_LUMA_SPREAD: f64 = 12.0;

/// Where the frames are kept: `<cache>/frames`, beside the image cache and
/// outside it, so the image cache's sweep of stray files never takes them.
/// Clear images removes it along with the image cache.
pub fn dir(paths: &anibeam_foundation::paths::CorePaths) -> PathBuf {
    paths.frames_dir()
}

/// The frame's file for a video at `path` of this size and mtime: FNV-1a over
/// all three, so the name is the same on every launch and a file replaced in
/// place gets a new one.
pub fn file_for(dir: &Path, path: &str, size: u64, mtime: i64) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(path.as_bytes());
    eat(&[0]);
    eat(&size.to_le_bytes());
    eat(&mtime.to_le_bytes());
    dir.join(format!("{h:016x}.jpg"))
}

/// The empty file beside a frame's name that says no frame came of that
/// video: no sample scored, or ffmpeg could not read it. It keeps a dark or
/// a broken episode from costing nine ffmpeg runs on every launch, and goes
/// with its video's size and mtime the same as the frame.
pub fn miss_for(frame: &Path) -> PathBuf {
    frame.with_extension("miss")
}

/// How interesting a frame is, from its RGB bytes (`rgb24`, rows top to
/// bottom), or None for one never worth showing: too dark, too bright or
/// flat. The score is colourfulness (Hasler and Süsstrunk's) times sharpness
/// (the variance of a Laplacian over luma), with one added to the
/// colourfulness so that a sharp frame with no colour in it still ranks.
pub fn interest(rgb: &[u8], width: usize, height: usize) -> Option<f64> {
    let n = width.checked_mul(height)?;
    if width < 3 || height < 3 || rgb.len() < n * 3 {
        return None;
    }
    let mut luma = Vec::with_capacity(n);
    let (mut sum_rg, mut sum_yb, mut sq_rg, mut sq_yb) = (0.0, 0.0, 0.0, 0.0);
    let (mut sum_y, mut sq_y) = (0.0, 0.0);
    for px in rgb[..n * 3].as_chunks::<3>().0 {
        let (r, g, b) = (f64::from(px[0]), f64::from(px[1]), f64::from(px[2]));
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        luma.push(y);
        sum_y += y;
        sq_y += y * y;
        let rg = r - g;
        let yb = 0.5 * (r + g) - b;
        sum_rg += rg;
        sum_yb += yb;
        sq_rg += rg * rg;
        sq_yb += yb * yb;
    }
    let count = n as f64;
    let mean_y = sum_y / count;
    let spread_y = (sq_y / count - mean_y * mean_y).max(0.0).sqrt();
    if mean_y < LUMA_RANGE.0 || mean_y > LUMA_RANGE.1 || spread_y < MIN_LUMA_SPREAD {
        return None;
    }
    let (mean_rg, mean_yb) = (sum_rg / count, sum_yb / count);
    let var_rg = (sq_rg / count - mean_rg * mean_rg).max(0.0);
    let var_yb = (sq_yb / count - mean_yb * mean_yb).max(0.0);
    let colourful = (var_rg + var_yb).sqrt() + 0.3 * (mean_rg * mean_rg + mean_yb * mean_yb).sqrt();

    let (mut sum_l, mut sq_l, mut inner) = (0.0, 0.0, 0.0);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let at = y * width + x;
            let lap =
                luma[at - 1] + luma[at + 1] + luma[at - width] + luma[at + width] - 4.0 * luma[at];
            sum_l += lap;
            sq_l += lap * lap;
            inner += 1.0;
        }
    }
    let mean_l = sum_l / inner;
    let sharp = (sq_l / inner - mean_l * mean_l).max(0.0);
    Some((1.0 + colourful) * sharp)
}

/// The sample times, in seconds, for a runtime of `duration` seconds.
pub fn sample_times(duration: f64) -> Vec<f64> {
    (0..SAMPLES)
        .map(|i| {
            let at = SPREAD.0 + (SPREAD.1 - SPREAD.0) * i as f64 / (SAMPLES - 1) as f64;
            duration * at
        })
        .collect()
}

/// Whether Home puts this series in Continue at all, the same test as its
/// `startedOf` (`HomePage.qml`): never a Completed, Dropped or Paused entry;
/// otherwise played here, or with episodes seen on the tracker while it is
/// being watched or rewatched there.
pub fn started(card: &SeriesCard) -> bool {
    match card.list_status {
        Some(ListStatus::Completed | ListStatus::Dropped | ListStatus::Paused) => false,
        status => {
            card.last_viewed_at.is_some()
                || (card.watched.is_some_and(|w| w > 0)
                    && matches!(status, Some(ListStatus::Watching | ListStatus::Repeating)))
        }
    }
}

/// The episode a Continue card for this series shows, as an index into
/// `detail.episodes`: the last one stopped short and not watched, else the
/// next one up; None for a series Home leaves out of Continue ([`started`]),
/// or when that episode is still being written. Home's own pick
/// (`HomePage.qml`, `startedOf` then `pickContinue`) reads the same way.
pub fn continue_episode(detail: &SeriesDetail) -> Option<usize> {
    if !started(&detail.card) {
        return None;
    }
    let (mut partial, mut next) = (None, None);
    for (i, e) in detail.episodes.iter().enumerate() {
        if e.downloading.is_some() {
            continue;
        }
        if !e.watched && e.stopped_short && e.resume.as_ref().is_some_and(|r| r.duration > 0.0) {
            partial = Some(i);
        }
        if detail.next_up == Some(e.file) {
            next = Some(i);
        }
    }
    partial.or(next)
}

/// One episode wanting a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    pub series: u64,
    pub source: PathBuf,
    pub out: PathBuf,
}

/// Lays the cached frame on the episode Continue would show and says what it
/// still wants: that episode, with no still, no frame on disk yet and no miss
/// remembered ([`miss_for`]).
/// `stamps` holds each of the series' files' size and mtime by file id, and
/// `stills_coming` the files whose still is known and not cached yet: those
/// wait for the still and take no frame.
pub fn lay_on(
    detail: &mut SeriesDetail,
    stamps: &HashMap<u64, (u64, i64)>,
    stills_coming: &HashSet<u64>,
    dir: &Path,
) -> Option<Want> {
    let at = continue_episode(detail)?;
    let series = detail.card.id;
    let e = &mut detail.episodes[at];
    if e.still.is_some() || stills_coming.contains(&e.file) {
        return None;
    }
    let (size, mtime) = *stamps.get(&e.file)?;
    let out = file_for(dir, &e.path, size, mtime);
    if out.is_file() {
        e.frame = Some(out.to_string_lossy().into_owned());
        return None;
    }
    if miss_for(&out).is_file() {
        return None;
    }
    Some(Want {
        series,
        source: PathBuf::from(&e.path),
        out,
    })
}

/// The queue of episodes wanting a frame and the one worker that takes them,
/// one ffmpeg at a time.
pub struct Frames {
    services: Arc<Services>,
    closed: Arc<AtomicBool>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Want>,
    /// Every frame asked for this launch: one that failed is not tried again
    /// until the next.
    asked: HashSet<PathBuf>,
    running: bool,
    /// ffmpeg or ffprobe is not here: nothing more is asked of them.
    missing: bool,
}

/// Why a frame was not written.
#[derive(Debug)]
enum Failed {
    /// ffmpeg or ffprobe could not be started at all.
    Missing,
    Other(String),
}

impl Frames {
    pub fn new(services: Arc<Services>, closed: Arc<AtomicBool>) -> Arc<Frames> {
        Arc::new(Frames {
            services,
            closed,
            state: Mutex::new(State::default()),
        })
    }

    /// Queues `want` unless it was asked for already this launch, and starts
    /// the worker when it is not running. A lock and maybe a spawn: a read
    /// pays nothing more.
    pub fn want(self: &Arc<Self>, want: Want) {
        let mut state = recover(&self.state);
        if state.missing || !state.asked.insert(want.out.clone()) {
            return;
        }
        state.queue.push_back(want);
        if state.running {
            return;
        }
        state.running = true;
        drop(state);
        let me = self.clone();
        self.services.handle.spawn(async move { me.work().await });
    }

    /// Takes the queue one frame at a time. The series whose frames landed
    /// are announced together: when the queue runs dry, or every
    /// [`ANNOUNCE_EVERY`] while it is long, so a launch with many bare
    /// Continue episodes reloads Home a few times rather than once a frame.
    async fn work(self: Arc<Self>) {
        let mut landed: Vec<u64> = Vec::new();
        let mut since = Instant::now();
        loop {
            let (next, more) = {
                let mut state = recover(&self.state);
                let next = if self.closed.load(Ordering::SeqCst) {
                    None
                } else {
                    state.queue.pop_front()
                };
                if next.is_none() {
                    state.running = false;
                }
                (next, !state.queue.is_empty())
            };
            let Some(want) = next else {
                self.announce(std::mem::take(&mut landed)).await;
                return;
            };
            let outcome = extract(&want.source, &want.out).await;
            match outcome {
                Ok(Some(_)) => {
                    if !landed.contains(&want.series) {
                        landed.push(want.series);
                    }
                }
                Ok(None) => {
                    tracing::debug!("no frame of {} was worth keeping", want.source.display());
                    remember_miss(&want.out);
                }
                Err(Failed::Missing) => {
                    {
                        let mut state = recover(&self.state);
                        state.missing = true;
                        state.queue.clear();
                        state.running = false;
                    }
                    tracing::info!("episode frames are off: ffmpeg or ffprobe is not installed");
                    self.announce(landed).await;
                    return;
                }
                Err(Failed::Other(e)) => {
                    tracing::debug!("no frame of {}: {e}", want.source.display());
                    remember_miss(&want.out);
                }
            }
            if !landed.is_empty() && (!more || since.elapsed() >= ANNOUNCE_EVERY) {
                self.announce(std::mem::take(&mut landed)).await;
                since = Instant::now();
            }
        }
    }

    /// The series' cards again, one event for all of them, so the shells
    /// read them once more and find the frames. Debug: a frame landing is
    /// not a change of state the activity log keeps.
    async fn announce(&self, series: Vec<u64>) {
        if series.is_empty() {
            return;
        }
        let images_dir = self.services.paths.images_dir();
        let services = self.services.clone();
        let ids = series.clone();
        // Only a read, so off the writer thread: it never waits behind a scan
        let cards = tokio::task::spawn_blocking(move || {
            services
                .store
                .read(|c| cards::cards_for(c, &images_dir, &ids))
        })
        .await
        .unwrap_or_else(|e| Err(CoreError::internal(format!("card read failed: {e}"))));
        match cards {
            Ok(cards) => {
                self.services.bus.debug(
                    Stage::Library,
                    format!("frames taken for {} series", series.len()),
                    EventBody::SeriesChanged { series: cards },
                );
            }
            Err(e) => tracing::warn!("frames for series {series:?} not announced: {e}"),
        }
    }
}

/// Writes the miss marker for `frame`, so later launches ask nothing of that
/// video. A marker that cannot be written only costs a retry next launch.
fn remember_miss(frame: &Path) {
    let marker = miss_for(frame);
    let wrote = marker
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&marker, []));
    if let Err(e) = wrote {
        tracing::debug!("miss for {} not kept: {e}", frame.display());
    }
}

/// Whether `nice` can be started here. Android, minimal containers and some
/// Flatpak runtimes have none; there the helper runs at its normal priority
/// rather than being taken for a missing ffmpeg. Asked once a launch.
#[cfg(unix)]
fn has_nice() -> bool {
    static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAS.get_or_init(|| {
        std::process::Command::new("nice")
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    })
}

/// A command for a helper at the lowest priority: under `nice` on Unix where
/// there is one, with a below-normal priority class on Windows.
fn low(program: &str) -> Command {
    #[cfg(unix)]
    {
        if !has_nice() {
            return command(program);
        }
        let mut cmd = command("nice");
        cmd.args(["-n", "19", program]);
        cmd
    }
    #[cfg(windows)]
    {
        /// BELOW_NORMAL_PRIORITY_CLASS, from WinBase.h.
        const BELOW_NORMAL: u32 = 0x0000_4000;
        let mut cmd = command(program);
        cmd.creation_flags(anibeam_foundation::process::NO_WINDOW | BELOW_NORMAL);
        cmd
    }
    #[cfg(not(any(unix, windows)))]
    command(program)
}

/// Runs `cmd` to its end within [`RUN_LIMIT`] and hands back its stdout.
/// `nice` exits 127 when the program it was asked to start is not there,
/// which counts as missing the same as a spawn that fails.
async fn run(mut cmd: Command) -> Result<Vec<u8>, Failed> {
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(RUN_LIMIT, out).await {
        Err(_) => return Err(Failed::Other("timed out".into())),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failed::Missing),
        Ok(Err(e)) => return Err(Failed::Other(e.to_string())),
        Ok(Ok(out)) => out,
    };
    match out.status.code() {
        Some(0) => Ok(out.stdout),
        Some(127) if cfg!(unix) => Err(Failed::Missing),
        code => Err(Failed::Other(format!("exited with {code:?}"))),
    }
}

/// The runtime in seconds, off ffprobe.
async fn duration(source: &Path) -> Result<f64, Failed> {
    let mut cmd = low("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ])
    .arg(source);
    let out = run(cmd).await?;
    String::from_utf8_lossy(&out)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or_else(|| Failed::Other("no duration".into()))
}

/// ffmpeg's arguments for one frame at `at` seconds: seeking before the input
/// is fast and lands on the frame at that time. `0:V:0` is the first video
/// stream that is not an attached picture, so a cover in an MKV or an MP4
/// is never what gets scored.
fn frame_args(source: &Path, at: f64) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin"]
        .iter()
        .map(Into::into)
        .collect();
    args.push("-ss".into());
    args.push(format!("{at:.3}").into());
    args.push("-i".into());
    args.push(source.into());
    args.extend(
        ["-map", "0:V:0", "-an", "-sn", "-frames:v", "1"]
            .iter()
            .map(Into::into),
    );
    args
}

/// One sample, decoded small as RGB: the bytes and the height.
async fn sample(source: &Path, at: f64) -> Result<(Vec<u8>, usize), Failed> {
    let mut cmd = low("ffmpeg");
    cmd.args(frame_args(source, at)).args([
        "-vf",
        &format!("scale={SAMPLE_WIDTH}:-2"),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
    ]);
    let rgb = run(cmd).await?;
    let height = rgb.len() / (SAMPLE_WIDTH * 3);
    if height < 3 {
        return Err(Failed::Other(format!("no frame at {at:.1}s")));
    }
    Ok((rgb, height))
}

/// Scores the samples of `source` and writes the best to `out`: the time it
/// was taken at, or None when no sample was worth keeping.
async fn extract(source: &Path, out: &Path) -> Result<Option<f64>, Failed> {
    let runtime = duration(source).await?;
    let mut best: Option<(f64, f64)> = None;
    for at in sample_times(runtime) {
        let (rgb, height) = match sample(source, at).await {
            Ok(s) => s,
            Err(Failed::Missing) => return Err(Failed::Missing),
            Err(Failed::Other(_)) => continue,
        };
        if let Some(score) = interest(&rgb, SAMPLE_WIDTH, height)
            && best.is_none_or(|(s, _)| score > s)
        {
            best = Some((score, at));
        }
    }
    let Some((_, at)) = best else {
        return Ok(None);
    };
    let dir = out
        .parent()
        .ok_or_else(|| Failed::Other("no frames directory".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| Failed::Other(e.to_string()))?;
    let part = out.with_extension("part.jpg");
    let mut cmd = low("ffmpeg");
    cmd.args(frame_args(source, at))
        .args([
            "-vf",
            &format!("scale='min({FRAME_WIDTH},iw)':-2"),
            "-q:v",
            "3",
            "-f",
            "image2",
            "-update",
            "1",
            "-y",
        ])
        .arg(&part);
    if let Err(e) = run(cmd).await {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, out).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        Failed::Other(e.to_string())
    })?;
    Ok(Some(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 64;
    const H: usize = 36;

    /// A frame of one colour.
    fn flat(r: u8, g: u8, b: u8) -> Vec<u8> {
        [r, g, b].repeat(W * H)
    }

    /// A frame of noise from a fixed seed: colourful, or grey when `grey`.
    fn noise(seed: u64, grey: bool) -> Vec<u8> {
        let mut s = seed;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) as u8
        };
        let mut out = Vec::with_capacity(W * H * 3);
        for _ in 0..W * H {
            if grey {
                let v = next();
                out.extend([v, v, v]);
            } else {
                out.extend([next(), next(), next()]);
            }
        }
        out
    }

    /// A smooth left to right ramp from red to green: colourful, not sharp.
    fn ramp() -> Vec<u8> {
        let mut out = Vec::with_capacity(W * H * 3);
        for _ in 0..H {
            for x in 0..W {
                let t = (x * 255 / (W - 1)) as u8;
                out.extend([255 - t, t, 60]);
            }
        }
        out
    }

    #[test]
    fn a_black_a_white_and_a_flat_grey_frame_are_never_worth_showing() {
        assert_eq!(interest(&flat(0, 0, 0), W, H), None);
        assert_eq!(interest(&flat(8, 10, 12), W, H), None);
        assert_eq!(interest(&flat(250, 250, 250), W, H), None);
        assert_eq!(interest(&flat(128, 128, 128), W, H), None);
        assert_eq!(interest(&flat(200, 40, 40), W, H), None);
    }

    #[test]
    fn a_colourful_sharp_frame_beats_a_grey_sharp_one_and_a_smooth_one() {
        let colour = interest(&noise(7, false), W, H).unwrap();
        let grey = interest(&noise(7, true), W, H).unwrap();
        let smooth = interest(&ramp(), W, H).unwrap();
        assert!(colour > grey, "{colour} vs {grey}");
        assert!(colour > smooth, "{colour} vs {smooth}");
    }

    #[test]
    fn a_short_buffer_or_a_tiny_frame_scores_nothing() {
        assert_eq!(interest(&noise(1, false)[..10], W, H), None);
        assert_eq!(interest(&[10, 200, 30, 90, 80, 70], 2, 1), None);
    }

    #[test]
    fn the_samples_spread_over_the_middle_of_the_runtime() {
        let at = sample_times(1000.0);
        assert_eq!(at.len(), SAMPLES);
        assert!((at[0] - 300.0).abs() < 1e-9);
        assert!((at[SAMPLES - 1] - 700.0).abs() < 1e-9);
        assert!(at.windows(2).all(|w| w[0] < w[1]));
    }

    /// A 20 s clip, black but for four colourful seconds from 8 s, made with
    /// ffmpeg's own sources; None without ffmpeg.
    fn clip(dir: &Path) -> Option<PathBuf> {
        let path = dir.join("episode.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(["-f", "lavfi", "-i", "color=black:s=320x180:r=25:d=8"])
            .args(["-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=4"])
            .args(["-f", "lavfi", "-i", "color=black:s=320x180:r=25:d=8"])
            .args([
                "-filter_complex",
                "[0][1][2]concat=n=3:v=1:a=0,format=yuv420p",
            ])
            .args(["-c:v", "libx264"])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        made.then_some(path)
    }

    #[tokio::test]
    async fn the_frame_written_is_one_of_the_colourful_samples_never_a_black_one() {
        let tmp = tempfile::tempdir().unwrap();
        let Some(source) = clip(tmp.path()) else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let out = file_for(&tmp.path().join("frames"), &source.to_string_lossy(), 1, 1);
        let at = extract(&source, &out).await.unwrap().expect("a frame");
        assert!((8.0..12.0).contains(&at), "taken at {at}");
        let jpeg = std::fs::read(&out).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8], "not a JPEG");
        assert!(!out.with_extension("part.jpg").exists());
        // Kept for a look by hand when asked
        if let Some(keep) = std::env::var_os("ANIBEAM_KEEP_FRAME") {
            std::fs::copy(&out, &keep).unwrap();
            eprintln!(
                "frame taken at {at:.2}s kept at {}",
                Path::new(&keep).display()
            );
        }
    }

    #[tokio::test]
    async fn a_clip_black_all_through_gives_no_frame() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("black.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
            .args(["-f", "lavfi", "-i", "color=black:s=320x180:r=25:d=6"])
            .args(["-c:v", "libx264"])
            .arg(&source)
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let out = tmp.path().join("frames").join("x.jpg");
        assert_eq!(extract(&source, &out).await.unwrap(), None);
        assert!(!out.exists());
    }

    #[test]
    fn a_miss_is_kept_beside_the_frames_name_and_never_taken_for_a_frame() {
        let tmp = tempfile::tempdir().unwrap();
        let out = file_for(&tmp.path().join("frames"), "/lib/Show/01.mkv", 100, 5);
        remember_miss(&out);
        let marker = miss_for(&out);
        assert!(marker.is_file());
        assert_eq!(std::fs::metadata(&marker).unwrap().len(), 0);
        assert!(!out.exists());
        assert_eq!(marker.parent(), out.parent());
    }

    #[test]
    fn a_frame_name_is_stable_and_moves_with_the_files_size_or_mtime() {
        let dir = Path::new("cache/frames");
        let a = file_for(dir, "/lib/Show/01.mkv", 100, 5);
        assert_eq!(a, file_for(dir, "/lib/Show/01.mkv", 100, 5));
        assert_ne!(a, file_for(dir, "/lib/Show/01.mkv", 101, 5));
        assert_ne!(a, file_for(dir, "/lib/Show/01.mkv", 100, 6));
        assert_ne!(a, file_for(dir, "/lib/Show/02.mkv", 100, 5));
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("jpg"));
    }
}
