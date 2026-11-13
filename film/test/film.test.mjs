import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

import { assertFrames, assertLivelyStart, BLANK_BYTES, blankBytes, collapse, isBlank, JOIN_FPS, MOVING_CAP_INTERVALS, pngSize, REFERENCE_PIXELS, trimStart, createFilm, HOLD_S, FILM_SIZE, FPS, frameList, framesInList, joinArgs, MIN_FRAMES } from "../film.mjs";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

const big = (n) => Buffer.alloc(BLANK_BYTES * 2, n).toString("base64");

describe("the frames of the film", () => {
  const clock = () => {
    let t = 0;
    return { now: () => t, sleep: async (ms) => void (t += ms), advance: (ms) => void (t += ms) };
  };
  it("takes a screenshot of the window at a steady rate until stopped, and names the frames in order", async () => {
    const saved = [];
    const c = clock();
    let shots = 0;
    const film = createFilm({
      dir: "out",
      shoot: async () => {
        shots += 1;
        if (shots === 6) film.cancel();
        return Buffer.from(`png${shots}`).toString("base64");
      },
      now: c.now,
      sleep: c.sleep,
      save: (file, bytes) => saved.push([file, bytes?.toString?.()]),
      makeDir: () => undefined,
    });
    film.start();
    await film.done;
    assert.equal(film.count, 6);
    assert.deepEqual(saved[0], ["out/frames/frame-00001.png", "png1"]);
    assert.equal(film.frames[1].at - film.frames[0].at, 1000 / FPS);
  });
  it("skips a screenshot the window could not give instead of ending the film", async () => {
    const c = clock();
    let n = 0;
    const film = createFilm({
      dir: "out",
      shoot: async () => {
        n += 1;
        if (n === 2) throw new Error("mid-transition");
        if (n === 4) film.cancel();
        return "AA==";
      },
      now: c.now,
      sleep: c.sleep,
      save: () => undefined,
      makeDir: () => undefined,
    });
    film.start();
    await film.done;
    assert.equal(film.count, 3);
  });
  it("films nothing without a directory", async () => {
    const film = createFilm({ shoot: async () => assert.fail("no film, no screenshot") });
    film.start();
    await film.stop();
    assert.equal(film.finish(), 0);
  });
  it("fails loudly when it holds fewer frames than the minimum (no empty clip, no green job without a film)", () => {
    assert.throws(() => assertFrames(0), /filmed nothing/);
    assert.throws(() => assertFrames(MIN_FRAMES - 1), /at least 30/);
    assert.equal(assertFrames(MIN_FRAMES), MIN_FRAMES);
    const film = createFilm({ dir: "out", shoot: async () => "AA==", save: () => undefined, makeDir: () => undefined });
    assert.throws(() => film.finish(), /filmed nothing/);
  });
  it("writes the frame list only once the minimum is met", async () => {
    const written = [];
    const c = clock();
    let n = 0;
    const film = createFilm({
      dir: "out",
      shoot: async () => {
        n += 1;
        if (n === 3) film.cancel();
        return big(n);
      },
      now: c.now,
      sleep: c.sleep,
      save: (file) => written.push(file),
      makeDir: () => undefined,
    });
    film.start();
    await film.done;
    assert.equal(film.finish(3), 3);
    assert.ok(written.includes("out/film-frames.txt"));
  });
});

describe("the frame list", () => {
  it("shows each frame until the next, never longer than three intervals (a stopped take's gap is not played), the last once more", () => {
    const f = (file, at, hash, size = 9000) => ({ file, at, hash, size });
    const text = frameList([f("frames/a.png", 0, "a"), f("frames/b.png", 100, "b"), f("frames/c.png", 60_000, "c")]);
    assert.equal(text, "file 'frames/a.png'\nduration 0.100\nfile 'frames/b.png'\nduration 0.300\nfile 'frames/c.png'\nduration 0.100\nfile 'frames/c.png'\n");
    assert.equal(framesInList(text), 3);
    assert.equal(framesInList(""), 0);
  });
});

describe("the start and the idle stretches of the film", () => {
  const f = (n, at, hash, size = 9000) => ({ file: `frames/${n}.png`, at, hash, size });
  it("cuts the blank window before the first screen that shows something", () => {
    const entries = collapse([f(1, 0, "x", 300), f(2, 100, "x", 300), f(3, 200, "a"), f(4, 300, "b")]);
    assert.deepEqual(entries.map((e) => e.file), ["frames/3.png", "frames/4.png"]);
  });
  it("shows an idle screen for HOLD_S at most, however long it waited, and keeps the pace of a moving one", () => {
    const idle = Array.from({ length: 50 }, (_, i) => f(i, i * 100, "same"));
    const [entry] = collapse([...idle, f(99, 5000, "next")]);
    assert.equal(entry.seconds, HOLD_S);
    const moving = collapse([f(1, 0, "a"), f(2, 100, "b"), f(3, 200, "c")]);
    assert.deepEqual(moving.map((e) => e.seconds.toFixed(1)), ["0.1", "0.1", "0.1"]);
  });
  it("refuses a film that opens blank, or on one picture only (idle), and accepts one that moves", () => {
    assert.throws(() => assertLivelyStart([f(1, 0, "a", 100), f(2, 100, "b")]), /blank/);
    assert.throws(() => assertLivelyStart([f(1, 0, "a"), f(2, 100, "a"), f(3, 200, "a")], 3), /same picture/);
    assert.throws(() => assertLivelyStart([f(1, 0, "a")]), /at least 2/);
    assert.doesNotThrow(() => assertLivelyStart([f(1, 0, "a"), f(2, 100, "b")]));
  });
  it("refuses a recorded film that never changed", async () => {
    const film = createFilm({ dir: "out", shoot: async () => big(7), now: () => 0, sleep: async () => film.cancel(), save: () => undefined, makeDir: () => undefined });
    film.start();
    await film.done;
    assert.throws(() => film.finish(1), /at least 2/);
  });
});

describe("the joined film", () => {
  it("is one silent mp4 of the frame list, letterboxed to one size", () => {
    const args = joinArgs("film-frames.txt", "film.mp4");
    assert.deepEqual(args.slice(0, 5), ["-y", "-f", "concat", "-i", "film-frames.txt"]);
    assert.match(args[args.indexOf("-vf") + 1], new RegExp(`scale=${FILM_SIZE.width}:${FILM_SIZE.height}:force_original_aspect_ratio=decrease`));
    assert.ok(args.includes("-an") && args.at(-1) === "film.mp4");
    assert.ok(!args.includes("-safe"), "the unsafe concat mode is not needed: the list holds relative paths");
    assert.match(args[args.indexOf("-vf") + 1], new RegExp(`fps=${JOIN_FPS}$`));
  });
  it("is joined by a script that refuses a film with too few frames and never screen-records", () => {
    assert.match(read("../join-cli.mjs"), /assertFrames\(framesInList/);
    for (const file of ["../film.mjs", "../join-cli.mjs", "../webdriver.mjs", "../../.github/actions/film/action.yml"]) {
      assert.doesNotMatch(read(file).replace(/^\s*(\/\/|#).*$/gm, ""), /screencapture/, file);
    }
  });
});

describe("the blank frame check follows the pixel count", () => {
  const png = (width, height, size) => {
    const bytes = Buffer.alloc(size);
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(bytes);
    bytes.writeUInt32BE(width, 16);
    bytes.writeUInt32BE(height, 20);
    return bytes;
  };
  it("reads the size of a PNG, and nothing from other bytes", () => {
    assert.deepEqual(pngSize(png(2560, 1600, 100)), { width: 2560, height: 1600 });
    assert.equal(pngSize(Buffer.from("not a png at all, but long enough to hold a header")), null);
  });
  it("at 1x, the threshold is BLANK_BYTES", () => {
    assert.equal(blankBytes(REFERENCE_PIXELS), BLANK_BYTES);
    assert.equal(isBlank({ size: BLANK_BYTES - 1, pixels: REFERENCE_PIXELS }), true);
    assert.equal(isBlank({ size: BLANK_BYTES, pixels: REFERENCE_PIXELS }), false);
  });
  it("at 2x, four times the pixels, the threshold is four times higher", () => {
    assert.equal(blankBytes(REFERENCE_PIXELS * 4), BLANK_BYTES * 4);
    assert.equal(isBlank({ size: BLANK_BYTES * 2, pixels: REFERENCE_PIXELS * 4 }), true, "a size that shows a 1x screen is blank at 2x");
    assert.equal(isBlank({ size: BLANK_BYTES * 4, pixels: REFERENCE_PIXELS * 4 }), false);
  });
  it("a frame of unknown size counts as the reference window", () => {
    assert.equal(isBlank({ size: BLANK_BYTES - 1 }), true);
    assert.equal(isBlank({ size: BLANK_BYTES }), false);
  });
  it("trims the blank start of a 2x film with the 2x threshold, and the recorder reads the pixels from the PNG", async () => {
    const f = (n, size) => ({ file: `frames/${n}.png`, at: n * 100, hash: String(n), size, pixels: REFERENCE_PIXELS * 4 });
    assert.deepEqual(trimStart([f(1, BLANK_BYTES * 2), f(2, BLANK_BYTES * 4)]).map((e) => e.file), ["frames/2.png"]);
    let n = 0;
    const film = createFilm({
      dir: "out",
      shoot: async () => {
        n += 1;
        if (n === 2) film.cancel();
        return png(2560, 1600, 20_000).toString("base64");
      },
      now: () => 0,
      sleep: async () => undefined,
      save: () => undefined,
      makeDir: () => undefined,
    });
    film.start();
    await film.done;
    assert.equal(film.frames[0].pixels, 2560 * 1600);
  });
  it("names the cap of a moving frame", () => {
    const entries = collapse([{ file: "a", at: 0, hash: "a", size: 9000 }, { file: "b", at: 60_000, hash: "b", size: 9000 }]);
    assert.equal(entries[0].seconds, (MOVING_CAP_INTERVALS * 100) / 1000);
  });
});
