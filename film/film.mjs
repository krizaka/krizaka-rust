// The Mac film (the Mac film): the journey films only the app's window, from the first screen of the library (after the sign-in,
// so no address is ever typed in front of the camera), skipping the engine's wait. The frames come from INSIDE the app: the same
// WebDriver the journey drives takes a screenshot of the window at a steady rate. No screen recording, so no macOS permission
// (TCC) is needed on the runner (`screencapture -v` filmed nothing there, runs 41071237202 and 41124359521).
// This file holds the parts that can be checked without a Mac: the frame recorder, the frame list, the join.
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// @fixed: ten frames a second reads as motion for a UI film and keeps the screenshots (about 100 ms each) in step
export const FPS = 10;
// @fixed: three seconds of film; fewer frames than this is a film that failed, never a clip to publish
export const MIN_FRAMES = 30;
// @fixed: the joined film is 1280 x 800, the app's window ratio, whatever the window is
export const FILM_SIZE = { width: 1280, height: 800 };
export const FRAMES_DIR = "frames";
export const FRAME_LIST = "film-frames.txt";

const frameName = (n) => `frame-${String(n).padStart(5, "0")}.png`;

/** Fails loudly when the film holds fewer frames than `min`: an empty film is an error, never a green job with no clip. */
export function assertFrames(count, min = MIN_FRAMES) {
  if (!Number.isInteger(count) || count < min) throw new Error(`the film holds ${count} frame(s), at least ${min} are needed: the journey filmed nothing`);
  return count;
}

// @fixed: a screen that does not change is shown this long (enough to read it), whatever the wait was: a brisk time-lapse
export const HOLD_S = 1.2;
// @fixed: a frame of the reference window (FILM_SIZE, 1x) under this size (PNG bytes) is one flat colour: a blank window or a
// splash, not a screen to show. A bigger frame (a Retina window at 2x) has more pixels, so its threshold grows with them.
export const BLANK_BYTES = 4000;
// @fixed: the pixel count BLANK_BYTES was measured on, the 1280 x 800 window at 1x
export const REFERENCE_PIXELS = 1280 * 800;
// @fixed: a single frame is shown for at most this many intervals: a stopped take's gap is not played
export const MOVING_CAP_INTERVALS = 3;
// @fixed: the joined film plays at 30 images a second, the rate players handle everywhere, whatever the capture rate was
export const JOIN_FPS = 30;

/** The width and height of a PNG, read from its header; `null` when the bytes are not a PNG. */
export function pngSize(bytes) {
  const signature = [0x89, 0x50, 0x4e, 0x47];
  if (bytes.length < 24 || signature.some((b, i) => bytes[i] !== b)) return null;
  return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) };
}

/** The size under which a frame of `pixels` pixels is blank (a frame whose size is unknown counts as the reference window). */
export const blankBytes = (pixels = REFERENCE_PIXELS) => Math.ceil((BLANK_BYTES * pixels) / REFERENCE_PIXELS);

/** Whether a frame `{ size, pixels? }` is one flat colour. */
export const isBlank = (frame) => frame.size < blankBytes(frame.pixels);

/** Leading frames that are blank are dropped: the film starts on the first screen that shows something. */
export function trimStart(frames) {
  const first = frames.findIndex((f) => !isBlank(f));
  return first < 0 ? [] : frames.slice(first);
}

/**
 * Whether the first `n` frames show something and move: they are not blank and not all the same picture. A film that opens on an
 * idle or empty window is refused (the clip starts on the first meaningful frame, see trimStart / still runs).
 */
export function assertLivelyStart(frames, n = 2) {
  const head = frames.slice(0, n);
  if (head.length < n) throw new Error(`the film opens on ${head.length} frame(s), at least ${n} are needed to judge its start`);
  if (head.some(isBlank)) throw new Error("the film opens on a blank window");
  if (new Set(head.map((f) => f.hash)).size < 2) throw new Error(`the first ${n} frames are the same picture: the film opens idle`);
}

/**
 * The collapsed film: the blank start is cut, and a screen that stays the same is one frame shown for at most HOLD_S (idle
 * stretches are dropped, a moving screen keeps its pace). `frames` = `[{ file, at, hash, size, pixels? }]`, `at` in milliseconds. 
 */
export function collapse(input, fps = FPS) {
  const interval = 1000 / fps;
  const frames = trimStart(input);
  const entries = [];
  for (let i = 0; i < frames.length; ) {
    let j = i;
    while (j + 1 < frames.length && frames[j + 1].hash === frames[i].hash) j += 1;
    const next = frames[j + 1];
    const span = (next ? next.at - frames[i].at : frames[j].at - frames[i].at + interval) / 1000;
    const moving = j === i;
    // a single frame keeps its own pace (at most MOVING_CAP_INTERVALS: a stopped take's gap is not played); a still run is held
    const seconds = moving ? Math.min(span, (MOVING_CAP_INTERVALS * interval) / 1000) : Math.min(span, HOLD_S);
    entries.push({ file: frames[i].file, seconds, hash: frames[i].hash, size: frames[i].size });
    i = j + 1;
  }
  return entries;
}

/** The concat list of the collapsed frames (the last file is named again, the demuxer ignoring the duration of the last one). */
export function frameList(input, fps = FPS) {
  const entries = collapse(input, fps);
  const lines = entries.flatMap((e) => [`file '${e.file}'`, `duration ${e.seconds.toFixed(3)}`]);
  if (entries.length > 0) lines.push(`file '${entries.at(-1).file}'`);
  return `${lines.join("\n")}\n`;
}

/**
 * The recorder: `start()` takes a screenshot of the window every 1/FPS s (`shoot()` answers a base64 PNG) until `stop()`; no
 * directory, no film. A screenshot that fails is skipped (the count tells); `finish()` writes the frame list and throws when
 * there are fewer than MIN_FRAMES.
 */
export function createFilm({
  dir,
  shoot,
  fps = FPS,
  now = Date.now,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)),
  save = writeFileSync,
  makeDir = (path) => mkdirSync(path, { recursive: true }),
} = {}) {
  const frames = [];
  const origin = now();
  const state = { running: false };
  let loop = Promise.resolve();
  const interval = 1000 / fps;
  async function run() {
    while (state.running) {
      const began = now();
      try {
        const png = await shoot();
        if (png) {
          const file = join(FRAMES_DIR, frameName(frames.length + 1));
          const bytes = Buffer.from(png, "base64");
          save(join(dir, file), bytes);
          const dimensions = pngSize(bytes);
          const pixels = dimensions ? dimensions.width * dimensions.height : undefined;
          frames.push({ file, at: began - origin, size: bytes.length, pixels, hash: createHash("sha256").update(bytes).digest("hex") });
        }
      } catch {
        // a screenshot the window could not give (a transition): the next one will do
      }
      await sleep(Math.max(0, interval - (now() - began)));
    }
  }
  return {
    frames,
    get count() {
      return frames.length;
    },
    start() {
      if (!dir || state.running) return;
      if (!shoot) throw new Error("the film needs a way to take a screenshot of the window");
      makeDir(join(dir, FRAMES_DIR));
      state.running = true;
      loop = run();
    },
    /** Resolves when the recording has ended (stopped or cancelled). */
    get done() {
      return loop;
    },
    /** Ends the recording at once without waiting for the frame in flight. */
    cancel() {
      state.running = false;
    },
    async stop() {
      state.running = false;
      await loop;
    },
    finish(min = MIN_FRAMES) {
      if (!dir) return 0;
      assertFrames(frames.length, min);
      assertLivelyStart(collapse(frames, fps));
      save(join(dir, FRAME_LIST), frameList(frames, fps));
      return frames.length;
    },
  };
}

/** `ffmpeg` arguments that turn the frame list into one mp4 of FILM_SIZE (letterboxed, never stretched), silent. The list names its frames by relative path, so ffmpeg's safe mode (the default) reads it. */
export function joinArgs(list, out) {
  const { width, height } = FILM_SIZE;
  const fit = `scale=${width}:${height}:force_original_aspect_ratio=decrease,pad=${width}:${height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=${JOIN_FPS}`;
  return ["-y", "-f", "concat", "-i", list, "-vf", fit, "-an", "-pix_fmt", "yuv420p", out];
}

/** The number of frames a frame list names (one `file` line per frame, the last one repeated for its duration). */
export const framesInList = (text) => Math.max(0, (text.match(/^file '/gm) ?? []).length - 1);
