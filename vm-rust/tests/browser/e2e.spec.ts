import { test, expect } from "@playwright/test";
import { spawnSync } from "child_process";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { PNG } from "pngjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SNAPSHOTS_BASE = path.join(__dirname, "..", "snapshots");
const UPDATE_SNAPSHOTS = process.env.SNAPSHOT_UPDATE === "1";

interface TestResult {
  name: string;
  /// "skip" is a test the runner never reached: a Rust panic left the wasm
  /// instance trapped, so it stopped rather than keep calling into it.
  status: "pass" | "fail" | "skip";
  error?: string;
}

interface TestResults {
  tests: TestResult[];
  passed: number;
  failed: number;
  /// Tests the runner never got to, because a Rust panic left the wasm
  /// instance trapped and it stopped rather than call into it again.
  skipped?: number;
  done: boolean;
}

function compareSnapshots(
  actualPath: string,
  referencePath: string,
  diffPath: string | null,
  pixelTolerance: number = 0
): { diffRatio: number; diffPixels: number; totalPixels: number } {
  const actual = PNG.sync.read(fs.readFileSync(actualPath));
  const reference = PNG.sync.read(fs.readFileSync(referencePath));

  if (actual.width !== reference.width || actual.height !== reference.height) {
    throw new Error(
      `Dimensions differ: actual ${actual.width}x${actual.height} vs reference ${reference.width}x${reference.height}`
    );
  }

  const totalPixels = actual.width * actual.height;
  let diffPixels = 0;

  // Build a diff image: changed pixels shown in red on a dimmed reference
  const diffImg = diffPath ? new PNG({ width: actual.width, height: actual.height }) : null;

  for (let i = 0; i < totalPixels; i++) {
    const off = i * 4;
    const dr = Math.abs(actual.data[off] - reference.data[off]);
    const dg = Math.abs(actual.data[off + 1] - reference.data[off + 1]);
    const db = Math.abs(actual.data[off + 2] - reference.data[off + 2]);
    const da = Math.abs(actual.data[off + 3] - reference.data[off + 3]);
    const changed = Math.max(dr, dg, db, da) > pixelTolerance;
    if (changed) diffPixels++;

    if (diffImg) {
      if (changed) {
        // Red highlight with intensity proportional to the diff
        diffImg.data[off] = 255;
        diffImg.data[off + 1] = 0;
        diffImg.data[off + 2] = 0;
        diffImg.data[off + 3] = 255;
      } else {
        // Dimmed reference pixel
        diffImg.data[off] = reference.data[off] >> 2;
        diffImg.data[off + 1] = reference.data[off + 1] >> 2;
        diffImg.data[off + 2] = reference.data[off + 2] >> 2;
        diffImg.data[off + 3] = reference.data[off + 3];
      }
    }
  }

  if (diffImg && diffPixels > 0 && diffPath) {
    fs.mkdirSync(path.dirname(diffPath), { recursive: true });
    fs.writeFileSync(diffPath, new Uint8Array(PNG.sync.write(diffImg)));
  } else if (diffPath && fs.existsSync(diffPath)) {
    fs.unlinkSync(diffPath);
  }

  return { diffRatio: diffPixels / totalPixels, diffPixels, totalPixels };
}

function processSnapshot(
  suitePath: string,
  name: string,
  base64data: string,
  maxDiffRatio: number,
  pixelTolerance: number = 0
): string {
  const slashIdx = suitePath.indexOf("/");
  const suite = slashIdx >= 0 ? suitePath.substring(0, slashIdx) : suitePath;
  const testName =
    slashIdx >= 0 ? suitePath.substring(slashIdx + 1) : "default";

  const outputDir = path.join(SNAPSHOTS_BASE, "output", suite, "browser", testName);
  const referenceDir = path.join(SNAPSHOTS_BASE, "reference", suite, "browser", testName);
  fs.mkdirSync(outputDir, { recursive: true });
  fs.mkdirSync(referenceDir, { recursive: true });

  const fileName = `${name}.png`;
  const outputPath = path.join(outputDir, fileName);
  const referencePath = path.join(referenceDir, fileName);

  fs.writeFileSync(outputPath, new Uint8Array(Buffer.from(base64data, "base64")));
  console.log(`Saved: ${suite}/browser/${testName}/${fileName}`);

  if (UPDATE_SNAPSHOTS) {
    fs.writeFileSync(
      referencePath,
      new Uint8Array(Buffer.from(base64data, "base64"))
    );
    return "reference updated";
  }

  if (!fs.existsSync(referencePath)) {
    return "no reference";
  }

  const diffDir = path.join(SNAPSHOTS_BASE, "diff", suite, "browser", testName);
  const diffPath = path.join(diffDir, fileName);
  const diff = compareSnapshots(outputPath, referencePath, diffPath, pixelTolerance);
  if (diff.diffRatio > maxDiffRatio) {
    throw new Error(
      `Snapshot '${suite}/browser/${testName}/${name}' differs from reference: ` +
        `${(diff.diffRatio * 100).toFixed(4)}% pixels changed ` +
        `(${diff.diffPixels}/${diff.totalPixels}, threshold: ${(maxDiffRatio * 100).toFixed(4)}%)`
    );
  }
  // Snapshot passed — remove any stale diff so the report doesn't flag it as changed.
  if (fs.existsSync(diffPath)) fs.unlinkSync(diffPath);
  return `${(diff.diffRatio * 100).toFixed(3)}% diff`;
}

// How many browser pages to split the corpus across. Each shard is its own
// page and therefore its own wasm instance, so sharding buys more than wall
// clock: a Rust panic can only take down the shard it happens in, and no single
// heap has to survive the whole corpus. A full sweep died at movie 48 inside
// dlmalloc's own heap-metadata assertion -- a cumulative-state failure that a
// smaller slice per page is much less likely to reach.
//
// Defaults to 1, i.e. exactly the old single-page behaviour. Raise it with
// `E2E_SHARDS=4 npm run e2e-test-browser`.
const SHARDS = Math.max(1, Number(process.env.E2E_SHARDS ?? 1) || 1);

if (SHARDS > 1) test.describe.configure({ mode: "parallel" });

for (let shard = 0; shard < SHARDS; shard++) {
test(SHARDS > 1 ? `browser e2e tests (shard ${shard + 1}/${SHARDS})` : "browser e2e tests", async ({ page }) => {
  const snapshotErrors: string[] = [];

  // Expose snapshot handler so snapshots are saved as they're taken
  await page.exposeFunction(
    "__playwrightSaveSnapshot",
    async (suite: string, name: string, data: string, maxDiffRatio: number, pixelTolerance: number = 0) => {
      try {
        const status = processSnapshot(suite, name, data, maxDiffRatio, pixelTolerance);
        return { ok: true, status };
      } catch (err: any) {
        const msg = err?.message ?? String(err);
        snapshotErrors.push(msg);
        return { ok: false, status: msg };
      }
    }
  );

  // `E2E_CONSOLE=1` forwards the page console to the terminal (optionally
  // filtered by a substring, e.g. `E2E_CONSOLE=PROBE`) — the only way to see
  // `log_test_action` / diagnostic output from inside the wasm test.
  // Every page console line is written to its own file under
  // test-results/console/, ALWAYS -- not only when E2E_CONSOLE is set.
  //
  // This is the artifact worth keeping from a sweep: the corpus emits a lot of
  // diagnostic traffic (unimplemented built-ins, value parsing failures,
  // missing member properties) that is the raw material for triage, and reading
  // it off interleaved stdout stopped being possible the moment shards began
  // running side by side. One file per shard keeps each stream in order and
  // attributable.
  //
  // A write stream rather than a buffer flushed at the end: the runs worth
  // reading are often the ones that die, and a crash must not take the log
  // with it.
  const consoleDir = path.resolve(__dirname, "../../..", "test-results", "console");
  fs.mkdirSync(consoleDir, { recursive: true });
  const logPath = path.join(
    consoleDir,
    SHARDS > 1 ? `shard${shard + 1}-of-${SHARDS}.log` : "console.log"
  );
  const logStream = fs.createWriteStream(logPath, { flags: "w" });

  // `E2E_CONSOLE=1` ALSO forwards to the terminal (optionally filtered by a
  // substring, e.g. `E2E_CONSOLE=PROBE`).
  const consoleFilter = process.env.E2E_CONSOLE;
  const needle = consoleFilter && consoleFilter !== "1" ? consoleFilter : "";
  page.on("console", (msg) => {
    const text = msg.text();
    logStream.write(text + "\n");
    if (consoleFilter && (!needle || text.includes(needle))) {
      console.log(`[page] ${text}`);
    }
  });
  // An uncaught page error never reaches `console`, and that is exactly what a
  // wasm trap surfaces as -- so it belongs in the log too.
  page.on("pageerror", (err) => {
    logStream.write(`[pageerror] ${err.message}\n`);
  });

  await page.goto(SHARDS > 1 ? `/index.html?shard=${shard}&shards=${SHARDS}` : "/index.html");

  // Wait for the harness to FINISH, or to declare itself aborted.
  //
  // This used to also stop on `__testPanic` being set, or on the first entry in
  // `__scriptErrors`. Both fire while the runner is still going, and because the
  // runner only published `__testResults` at the very end, the spec then read
  // null and reported "harness exited without publishing test results" --
  // throwing away every test that had already passed. A whole-suite run died
  // that way twice: an assert_eq! in rasterwerks_settings, and a dlmalloc
  // heap-metadata assertion 48 movies into the shared heap, the second of which
  // discarded 47 green results.
  //
  // The runner now owns both cases: it records a panic against the test that
  // raised it, marks the rest skipped, and sets `__testAborted`. Script errors
  // are still collected and reported below, they just no longer cut the run
  // short.
  const handle = await page.waitForFunction(
    () => {
      const win = window as any;
      return (
        win.__testResults?.done === true || typeof win.__testAborted === "string"
      );
    },
    // Just under the Playwright test timeout (5_400_000), so a slow-but-healthy
    // sweep reports through the normal path instead of being cut off here. The
    // old 900_000 was already below the wall time of a full run -- the `3d` tag
    // alone takes ~15.7 min -- so a complete suite could have tripped it while
    // making perfectly good progress.
    //
    // `undefined` for `arg`: the options are the THIRD parameter of
    // `waitForFunction`, and passing them second makes them the predicate's
    // argument instead -- silently reinstating the 30 s default timeout.
    undefined,
    { timeout: 5_220_000 }
  );
  await handle.dispose();

  const [testResults, panicMessage, abortMessage, scriptErrors, interpStats] = await Promise.all([
    page.evaluate(() => ((window as any).__testResults ?? null) as TestResults | null),
    page.evaluate(() => ((window as any).__testPanic ?? null) as string | null),
    page.evaluate(() => ((window as any).__testAborted ?? null) as string | null),
    page.evaluate(() => ((window as any).__scriptErrors ?? []) as string[]),
    page.evaluate(() => ((window as any).__interpStats ?? null) as string | null),
  ]);

  // Interpreter opcode/escape counters, accumulated across every test in the
  // suite (all of them share one wasm instance). Written unconditionally so a
  // failing run still yields the histogram.
  if (interpStats) {
    const statsDir = path.resolve(__dirname, "../../..", "test-results");
    fs.mkdirSync(statsDir, { recursive: true });
    const statsPath = path.join(
      statsDir,
      SHARDS > 1 ? `interp-stats-shard${shard + 1}.txt` : "interp-stats.txt"
    );
    fs.writeFileSync(statsPath, interpStats);
    console.log(`\nInterpreter stats written to ${statsPath}`);
    console.log(interpStats);
  }

  // Collect all errors before acting on them so keep-open can fire first.
  const errors: string[] = [];

  // A panic the runner already attributed to a test is reported through that
  // test's own result line, so mention it once here as the reason the run
  // stopped early. `panicMessage` on its own is the residue of a panic raised
  // OUTSIDE any test (module init, say), which no test result covers.
  if (abortMessage) {
    errors.push(`Run aborted: ${abortMessage}`);
  } else if (panicMessage) {
    errors.push(`Rust panic during browser test:\n${panicMessage}`);
  }

  if (scriptErrors.length > 0) {
    console.log(`\n${scriptErrors.length} script error(s):`);
    for (const err of scriptErrors) {
      console.log(`  ✗ ${err}`);
    }
    errors.push(`${scriptErrors.length} script error(s) during test:\n${scriptErrors.join("\n")}`);
  }

  if (!testResults) {
    errors.push("Browser test harness exited without publishing test results.");
  }

  if (testResults) {
    for (const t of testResults.tests) {
      if (t.status === "pass") {
        console.log(`✓ ${t.name}`);
      } else if (t.status === "skip") {
        console.log(`- ${t.name} (skipped): ${t.error}`);
      } else {
        console.log(`✗ ${t.name}: ${t.error}`);
      }
    }
    const skipped = testResults.skipped ?? 0;
    console.log(
      `${testResults.passed} passed, ${testResults.failed} failed` +
        (skipped ? `, ${skipped} skipped` : "")
    );
  }

  if (snapshotErrors.length > 0) {
    errors.push(
      `${snapshotErrors.length} snapshot comparison failure(s):\n${snapshotErrors.join("\n")}`
    );
  }

  // In debug mode, generate the snapshot report while the browser is still
  // open so the user doesn't need to Ctrl+C to trigger it.
  if (process.env.E2E_KEEP_OPEN === "1") {
    const repoRoot = path.resolve(__dirname, "../../..");
    spawnSync(
      "node",
      [
        path.join(repoRoot, "scripts", "generate-snapshot-report.mjs"),
        path.join(repoRoot, "vm-rust", "tests", "snapshots"),
        path.join(repoRoot, "test-results", "snapshot-report"),
      ],
      { stdio: "inherit" }
    );
  }

  // In debug mode, keep the browser open so the log can be inspected.
  if (process.env.E2E_KEEP_OPEN === "1" && errors.length > 0) {
    console.log("\nKeeping browser open for inspection — press Ctrl+C to exit.");
    await new Promise<void>(() => {});
  }

  // Flush the page log before anything can throw, so a failing run still
  // leaves a complete console capture behind.
  logStream.end();
  console.log(`Page console log: ${logPath}`);

  if (errors.length > 0) {
    throw new Error(errors.join("\n\n"));
  }

  // Assert all tests passed
  expect(testResults!.failed).toBe(0);
});
}
