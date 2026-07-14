/** The final segment of a slash- or backslash-separated path (the file/dir name). */
export function basename(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}
