export function fuzzyMatch(query: string, text: string): { score: number; matched: boolean } {
  if (!query) return { score: 1, matched: true };
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (t.includes(q)) return { score: q.length / t.length + 1, matched: true };
  let qi = 0;
  let score = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      score++;
      qi++;
    }
  }
  return { score: score / text.length, matched: qi === q.length };
}
