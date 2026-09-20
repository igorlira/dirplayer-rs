/**
 * Load a real corpus movie through the extension SW relay.
 *
 * Embed src is http://cdn.dirplayer.test/... so shouldRelayCrossOriginFetch
 * is true. Production load_movie_file → fetch_net_task → patched fetch → SW.
 * Success is a mounted stage canvas, not just Port framing.
 */
import {
  expect,
  gotoPlayerPage,
  isolatedPortLog,
  portKinds,
  test,
  waitForMovieLoad,
} from "./fixtures";
import {
  EMBED_FAST_PATH,
  EMBED_SLOW_PATH,
  SHAPES_PAYLOAD,
} from "./payloads.mjs";

test.describe.configure({ mode: "serial" });

test.describe("corpus movie via SW", () => {
  test.beforeEach(() => {
    test.skip(
      !SHAPES_PAYLOAD,
      "missing public/dcr_dirplayer_test_movies/D8_5_00001_shapes_1.dcr",
    );
  });

  test("single-chunk corpus movie still loads through the SW relay", async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await gotoPlayerPage(page, EMBED_FAST_PATH);
    await waitForMovieLoad(page);

    const log = await isolatedPortLog(page);
    const kinds = portKinds({ portLog: log });
    expect(kinds[0]).toBe("meta");
    expect(kinds).toContain("chunk");
    expect(kinds[kinds.length - 1]).toBe("done");
    const meta = log.find((e) => e.kind === "meta");
    expect(meta?.contentLength).toBe(String(SHAPES_PAYLOAD!.length));
  });

  test("streamed corpus movie still loads through the SW relay", async ({
    page,
  }) => {
    test.setTimeout(90_000);
    await gotoPlayerPage(page, EMBED_SLOW_PATH);
    await waitForMovieLoad(page);

    const log = await isolatedPortLog(page);
    const kinds = portKinds({ portLog: log });
    expect(kinds[0]).toBe("meta");
    expect(kinds.filter((k) => k === "chunk").length).toBeGreaterThanOrEqual(2);
    expect(kinds[kinds.length - 1]).toBe("done");
    const meta = log.find((e) => e.kind === "meta");
    expect(meta?.contentLength).toBe(String(SHAPES_PAYLOAD!.length));
  });
});
