import { export_cast } from 'vm-rust';
import { zipSync } from 'fflate';
import { downloadBlob } from './download';

type CastPackFile = { path: string; text: string } | { path: string; binary: Uint8Array };
type CastPackResult = { name: string; files: CastPackFile[] };

export function exportCastToZip(castNumber: number): void {
  const result = export_cast(castNumber) as CastPackResult | undefined;
  if (!result) {
    console.error(`export_cast returned nothing for cast ${castNumber}`);
    return;
  }

  const { name, files } = result;
  const encoder = new TextEncoder();
  const zipEntries: Record<string, Uint8Array> = {};

  for (const file of files) {
    if ('text' in file) {
      zipEntries[file.path] = encoder.encode(file.text);
    } else {
      zipEntries[file.path] = file.binary;
    }
  }

  const zipped = zipSync(zipEntries, { level: 6 });
  const safeName = name.replace(/[/\\:*?"<>|]/g, '_') || `cast_${castNumber}`;
  downloadBlob(zipped, `${safeName}.castpack.zip`);
}
