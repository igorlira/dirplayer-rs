export function getCaseInsensitiveValue(obj: Record<string, any>, key: string): string | undefined {
  for (const k in obj) {
    if (k.toLowerCase() === key.toLowerCase()) {
      return obj[k];
    }
  }
  return undefined;
}
