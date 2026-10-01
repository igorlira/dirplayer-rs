import { charKey, diffProxyValue, TEXT_PROXY_SENTINEL as S } from "./textProxy";

const keys = (before: string, after: string) =>
  diffProxyValue(before, after).map(k => k.key).join("|");

test("a typed character becomes one key", () => {
  expect(keys(S, S + "a")).toBe("a");
});

test("non-ASCII letters come through unchanged", () => {
  expect(keys(S, S + "ø")).toBe("ø");
  expect(charKey("Å")).toEqual({ key: "Å", code: "Å".charCodeAt(0) });
});

test("deleting the sentinel is a Backspace", () => {
  expect(keys(S, "")).toBe("Backspace");
});

test("a growing composition only delivers the new characters", () => {
  expect(keys(S + "he", S + "hej")).toBe("j");
});

test("an autocorrected word is replayed as Backspaces and the new text", () => {
  expect(keys(S + "teh", S + "the")).toBe("Backspace|Backspace|h|e");
});

test("a line break becomes Enter", () => {
  expect(diffProxyValue(S, S + "\n")).toEqual([{ key: "Enter", code: 13 }]);
});

test("no change delivers nothing", () => {
  expect(diffProxyValue(S + "x", S + "x")).toEqual([]);
});
