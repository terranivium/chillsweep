// Light/dark theme, ported from Vocal Slice's theme.js (Catppuccin Latte Soft / Mocha).
// Loaded as a plain script in <head> and applied to <html> straight away, so there is no flash
// of the wrong theme. It follows the system setting until the user presses the toggle.
(() => {
  const MODE_KEY = "themeMode"; // "light" | "dark"
  const THEMES = { light: "theme-latte-soft", dark: "theme-mocha" };
  const system = window.matchMedia("(prefers-color-scheme: dark)");

  function saved() {
    try {
      return localStorage.getItem(MODE_KEY);
    } catch {
      return null;
    }
  }

  function current() {
    return document.documentElement.classList.contains(THEMES.dark) ? "dark" : "light";
  }

  function apply(mode) {
    const root = document.documentElement;
    root.classList.remove(THEMES.light, THEMES.dark);
    root.classList.add(THEMES[mode]);
    const icon = document.getElementById("theme-icon");
    if (icon) icon.className = mode === "dark" ? "ph-fill ph-moon" : "ph-fill ph-sun";
    const button = document.getElementById("theme-toggle");
    if (button) button.title = mode === "dark" ? "Switch to light theme" : "Switch to dark theme";
  }

  function toggle() {
    const next = current() === "dark" ? "light" : "dark";
    try {
      localStorage.setItem(MODE_KEY, next);
    } catch {
      // Not remembered, but still switches for this session.
    }
    apply(next);
  }

  apply(saved() || (system.matches ? "dark" : "light"));
  system.addEventListener("change", (e) => {
    if (!saved()) apply(e.matches ? "dark" : "light");
  });
  window.addEventListener("DOMContentLoaded", () => {
    apply(current()); // sync the toggle icon now that it exists
    document.getElementById("theme-toggle")?.addEventListener("click", toggle);
  });
})();
