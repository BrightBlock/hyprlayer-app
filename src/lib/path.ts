/** The final segment of a forward-slashed path (the file/dir name). */
export function basename(path: string): string {
  return path.split("/").pop() || path;
}
