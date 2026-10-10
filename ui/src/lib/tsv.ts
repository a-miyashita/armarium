// TSV text for the clipboard, in the form that Excel uses.

export function tsvField(v: string): string {
  return /[\t\r\n"]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

export function toTsv(rows: string[][]): string {
  return rows.map((r) => r.map(tsvField).join("\t")).join("\r\n");
}
