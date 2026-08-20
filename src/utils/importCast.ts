import { import_cast_pack } from 'vm-rust';
import { unzipSync } from 'fflate';

type ImportResult = { ok: true; count: number } | { ok: false; error: string };

export async function importCastFromZip(castNumber: number, file: File): Promise<number> {
  const buffer = await file.arrayBuffer();
  const extracted = unzipSync(new Uint8Array(buffer));

  const files = Object.entries(extracted).map(([path, content]) => ({ path, content }));
  const result = import_cast_pack(castNumber, files) as ImportResult;

  if (!result.ok) {
    throw new Error(result.error);
  }
  return result.count;
}
