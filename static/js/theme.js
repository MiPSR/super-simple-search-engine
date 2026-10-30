// Apply the tiny browser-local theme preference before the first paint.
try {
  const theme = localStorage.getItem('theme');
  if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
} catch { /* Storage is optional. */ }
