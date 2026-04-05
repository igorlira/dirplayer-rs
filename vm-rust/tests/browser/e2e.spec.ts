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

  const diff = compareSnapshots(outputPath, referencePath);
  if (diff.diffRatio > maxDiffRatio) {
    throw new Error(
      `Snapshot '${suite}/browser/${testName}/${name}' differs from reference: ` +
        `${(diff.diffRatio * 100).toFixed(4)}% pixels changed ` +
        `(${diff.diffPixels}/${diff.totalPixels}, threshold: ${(maxDiffRatio * 100).toFixed(4)}%)`
    );
  }
}

test("browser e2e tests", async ({ page }) => {
  // Expose snapshot handler so snapshots are saved as they're taken
  await page.exposeFunction(
    "__playwrightSaveSnapshot",
    (suite: string, name: string, data: string, maxDiffRatio: number) => {
      processSnapshot(suite, name, data, maxDiffRatio);
    }
  );

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

  // Check for VM script errors
  const scriptErrors = await page.evaluate(
    () => (window as any).__scriptErrors as string[]
  );
  if (scriptErrors.length > 0) {
    console.log(`\n${scriptErrors.length} script error(s):`);
    for (const err of scriptErrors) {
      console.log(`  ✗ ${err}`);
    }
    throw new Error(
      `${scriptErrors.length} script error(s) during test:\n${scriptErrors.join("\n")}`
    );
  }

  // Assert all tests passed
  expect(testResults.failed).toBe(0);
});
