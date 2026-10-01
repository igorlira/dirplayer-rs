// Turning what an on-screen keyboard did to the hidden text proxy into key
// presses.
//
// Touch keyboards rarely report real keys: Android keyboards send keydown
// with key "Unidentified" (keyCode 229) and deliver the text only through
// input events, often inside a composition that grows and is rewritten word
// by word (autocorrect). So the proxy's value is compared with what was
// already delivered, and the difference is replayed as Backspaces followed by
// the new characters, which is exactly what a player typing on a physical
// keyboard would have produced.
//
// The proxy always holds a one-character sentinel. With an empty input a
// Backspace deletes nothing, so many keyboards fire no input event at all and
// the key would be lost; deleting the sentinel is a change we can see.

export const TEXT_PROXY_SENTINEL = " ";

export type ProxyKey = { key: string; code: number };

/** Keys that turn `before` into `after`, as Backspaces then characters. */
export function diffProxyValue(before: string, after: string): ProxyKey[] {
  const a = Array.from(before);
  const b = Array.from(after);
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  let suffix = 0;
  while (
    suffix < a.length - prefix &&
    suffix < b.length - prefix &&
    a[a.length - 1 - suffix] === b[b.length - 1 - suffix]
  ) suffix++;
  const removed = a.length - prefix - suffix;
  const added = b.slice(prefix, b.length - suffix);
  const keys: ProxyKey[] = [];
  for (let i = 0; i < removed; i++) keys.push({ key: "Backspace", code: 8 });
  for (const ch of added) keys.push(charKey(ch));
  return keys;
}

/** A typed character as the key the VM expects (JS keyCode convention). */
export function charKey(ch: string): ProxyKey {
  if (ch === "\n" || ch === "\r") return { key: "Enter", code: 13 };
  if (ch === "\t") return { key: "Tab", code: 9 };
  // toUpperCase().charCodeAt(0) matches the keyCode a physical key reports
  // for letters ('a' -> 65), the same mapping the desktop input path uses.
  return { key: ch, code: ch.toUpperCase().charCodeAt(0) };
}
