import { test, expect } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { PNG } from "pngjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SNAPSHOTS_BASE = path.join(__dirname, "..", "snapshots");
const UPDATE_SNAPSHOTS = process.env.SNAPSHOT_UPDATE === "1";
const MAX_DIFF_RATIO = parseFloat(process.env.SNAPSHOT_MAX_DIFF || "0.005");

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
  referencePath: string
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

  for (let i = 0; i < totalPixels; i++) {
    const off = i * 4;
    const dr = Math.abs(actual.data[off] - reference.data[off]);
    const dg = Math.abs(actual.data[off + 1] - reference.data[off + 1]);
    const db = Math.abs(actual.data[off + 2] - reference.data[off + 2]);
    const da = Math.abs(actual.data[off + 3] - reference.data[off + 3]);
    if (Math.max(dr, dg, db, da) > 0) diffPixels++;
  }

  return { diffRatio: diffPixels / totalPixels, diffPixels, totalPixels };
}

test("browser e2e tests", async ({ page }) => {
  await page.goto("/index.html");

  // Wait for all wasm tests to complete
  const handle = await page.waitForFunction(
    () => (window as any).__testResults as TestResults | null,
    { timeout: 120_000 }
  );
  const testResults = (await handle.jsonValue()) as TestResults;

  // Log results
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

  // Process snapshots
  const snapshots = await page.evaluate(
    () =>
      (window as any).__snapshots as {
        suite: string;
        name: string;
        data: string;
      }[]
  );

  for (const snap of snapshots) {
    // snap.suite is "suite/test" (e.g. "habbo/load")
    // Layout: snapshots/{suite}/browser/{test}/{name}.png
    const slashIdx = snap.suite.indexOf("/");
    const suite = slashIdx >= 0 ? snap.suite.substring(0, slashIdx) : snap.suite;
    const testName = slashIdx >= 0 ? snap.suite.substring(slashIdx + 1) : "default";

    const outputDir = path.join(
      SNAPSHOTS_BASE,
      "output",
      suite,
      "browser",
      testName
    );
    const referenceDir = path.join(
      SNAPSHOTS_BASE,
      "reference",
      suite,
      "browser",
      testName
    );
    fs.mkdirSync(outputDir, { recursive: true });
    fs.mkdirSync(referenceDir, { recursive: true });

    const fileName = `${snap.name}.png`;
    const outputPath = path.join(outputDir, fileName);
    const referencePath = path.join(referenceDir, fileName);

    fs.writeFileSync(
      outputPath,
      new Uint8Array(Buffer.from(snap.data, "base64"))
    );
    console.log(`Saved: ${suite}/browser/${testName}/${fileName}`);

    if (UPDATE_SNAPSHOTS) {
      fs.writeFileSync(
        referencePath,
        new Uint8Array(Buffer.from(snap.data, "base64"))
      );
      console.log(`Updated reference: ${suite}/browser/${testName}/${fileName}`);
      continue;
    }

    if (!fs.existsSync(referencePath)) {
      console.log(
        `No reference for '${suite}/browser/${testName}/${snap.name}'. Run with SNAPSHOT_UPDATE=1 to create.`
      );
      continue;
    }

    const diff = compareSnapshots(outputPath, referencePath);
    if (diff.diffRatio > MAX_DIFF_RATIO) {
      throw new Error(
        `Snapshot '${suite}/browser/${testName}/${snap.name}' differs from reference: ` +
          `${(diff.diffRatio * 100).toFixed(4)}% pixels changed ` +
          `(${diff.diffPixels}/${diff.totalPixels}, threshold: ${(MAX_DIFF_RATIO * 100).toFixed(4)}%)`
      );
    }
  }

  // Assert all tests passed
  expect(testResults.failed).toBe(0);
});
