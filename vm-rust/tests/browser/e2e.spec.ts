import { test, expect } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { PNG } from "pngjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SNAPSHOTS_BASE = path.join(__dirname, "..", "snapshots");
const UPDATE_SNAPSHOTS = process.env.SNAPSHOT_UPDATE === "1";

interface TestResult {
  name: string;
  status: "pass" | "fail";
  error?: string;
}

interface TestResults {
  tests: TestResult[];
  passed: number;
  failed: number;
  done: boolean;
}

function compareSnapshots(
  actualPath: string,
  referencePath: string,
  diffPath: string | null
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
    const changed = Math.max(dr, dg, db, da) > 0;
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
  }

  return { diffRatio: diffPixels / totalPixels, diffPixels, totalPixels };
}

function processSnapshot(
  suitePath: string,
  name: string,
  base64data: string,
  maxDiffRatio: number
) {
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
    console.log(`Updated reference: ${suite}/browser/${testName}/${fileName}`);
    return;
  }

  if (!fs.existsSync(referencePath)) {
    console.log(
      `No reference for '${suite}/browser/${testName}/${name}'. Run with SNAPSHOT_UPDATE=1 to create.`
    );
    return;
  }

  const diffDir = path.join(SNAPSHOTS_BASE, "diff", suite, "browser", testName);
  const diffPath = path.join(diffDir, fileName);
  const diff = compareSnapshots(outputPath, referencePath, diffPath);
  if (diff.diffRatio > maxDiffRatio) {
    throw new Error(
      `Snapshot '${suite}/browser/${testName}/${name}' differs from reference: ` +
        `${(diff.diffRatio * 100).toFixed(4)}% pixels changed ` +
        `(${diff.diffPixels}/${diff.totalPixels}, threshold: ${(maxDiffRatio * 100).toFixed(4)}%)`
    );
  }
}

test("browser e2e tests", async ({ page }) => {
  const snapshotErrors: string[] = [];

  // Expose snapshot handler so snapshots are saved as they're taken
  await page.exposeFunction(
    "__playwrightSaveSnapshot",
    (suite: string, name: string, data: string, maxDiffRatio: number) => {
      try {
        processSnapshot(suite, name, data, maxDiffRatio);
      } catch (err: any) {
        snapshotErrors.push(err?.message ?? String(err));
      }
    }
  );

  await page.goto("/index.html");

  // Stop waiting as soon as the harness finishes, a panic hook reports a
  // Rust panic, or the page accumulates script errors.
  const handle = await page.waitForFunction(
    () => {
      const win = window as any;
      return (
        win.__testResults?.done === true ||
        typeof win.__testPanic === "string" ||
        (Array.isArray(win.__scriptErrors) && win.__scriptErrors.length > 0)
      );
    },
    { timeout: 900_000 }
  );
  await handle.dispose();

  const [testResults, panicMessage, scriptErrors] = await Promise.all([
    page.evaluate(() => ((window as any).__testResults ?? null) as TestResults | null),
    page.evaluate(() => ((window as any).__testPanic ?? null) as string | null),
    page.evaluate(() => ((window as any).__scriptErrors ?? []) as string[]),
  ]);

  if (panicMessage) {
    throw new Error(`Rust panic during browser test:\n${panicMessage}`);
  }

  if (scriptErrors.length > 0) {
    console.log(`\n${scriptErrors.length} script error(s):`);
    for (const err of scriptErrors) {
      console.log(`  ✗ ${err}`);
    }
    throw new Error(
      `${scriptErrors.length} script error(s) during test:\n${scriptErrors.join("\n")}`
    );
  }

  if (!testResults) {
    throw new Error("Browser test harness exited without publishing test results.");
  }

  if (testResults) {
    for (const t of testResults.tests) {
      if (t.status === "pass") {
        console.log(`✓ ${t.name}`);
      } else {
        console.log(`✗ ${t.name}: ${t.error}`);
      }
    }
    console.log(
      `${testResults.passed} passed, ${testResults.failed} failed`
    );
  }

  if (snapshotErrors.length > 0) {
    throw new Error(
      `${snapshotErrors.length} snapshot comparison failure(s):\n${snapshotErrors.join("\n")}`
    );
  }

  // Assert all tests passed
  expect(testResults!.failed).toBe(0);
});
