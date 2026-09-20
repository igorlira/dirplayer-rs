import { base64ToBytes, bytesToBase64 } from './base64Binary';

describe('bytesToBase64 / base64ToBytes', () => {
  it('round-trips empty bytes', () => {
    expect(bytesToBase64(new Uint8Array(0))).toBe('');
    expect(Array.from(base64ToBytes(''))).toEqual([]);
  });

  it('round-trips ASCII and binary', () => {
    const ascii = new Uint8Array([72, 97, 116, 80, 67]); // HatPC
    expect(bytesToBase64(ascii)).toBe(btoa('HatPC'));
    expect(Array.from(base64ToBytes(bytesToBase64(ascii)))).toEqual(Array.from(ascii));

    const binary = new Uint8Array([0, 1, 127, 128, 255]);
    expect(Array.from(base64ToBytes(bytesToBase64(binary)))).toEqual(Array.from(binary));
  });

  it('round-trips buffers larger than the 0x8000 encode chunk', () => {
    const bytes = new Uint8Array(0x8000 + 17);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i & 0xff;
    expect(Array.from(base64ToBytes(bytesToBase64(bytes)))).toEqual(Array.from(bytes));
  });

  it('reassembles a body from sequential Port.postMessage chunks', () => {
    const body = new Uint8Array([0, 255, 1, 2, 3, 128]);
    const parts = [body.subarray(0, 2), body.subarray(2, 4), body.subarray(4)];
    const out = new Uint8Array(body.length);
    let offset = 0;
    for (const part of parts) {
      const decoded = base64ToBytes(bytesToBase64(part));
      out.set(decoded, offset);
      offset += decoded.length;
    }
    expect(Array.from(out)).toEqual(Array.from(body));
  });
});
