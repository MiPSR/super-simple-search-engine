export function getPref(key, def) {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return def;
    const n = Number(v);
    if (Number.isFinite(n) && n > 0) return n;
    return v;
  } catch (e) { return def; }
}
export function setPref(key, val) {
  try { localStorage.setItem(key, String(val)); } catch (e) {}
}
