export function fuzzyScore(text: string, q: string): number | null {
  if (!q) return 0;
  const t = text.toLowerCase();
  const needle = q.toLowerCase();
  let ti = 0;
  let score = 0;
  let run = 0;
  let matched = 0;
  for (let ni = 0; ni < needle.length; ni++) {
    const ch = needle[ni];
    let found = -1;
    for (let i = ti; i < t.length; i++) {
      if (t[i] === ch) {
        found = i;
        break;
      }
    }
    if (found === -1) return null;
    matched++;
    run = found === ti ? run + 1 : 0;
    score += 1 + run * 2;
    if (found === 0 || /\W|_/.test(t[found - 1])) score += 3;
    ti = found + 1;
  }
  if (t.startsWith(needle)) score += 5;
  score -= t.length * 0.01;
  return matched === needle.length ? score : null;
}
