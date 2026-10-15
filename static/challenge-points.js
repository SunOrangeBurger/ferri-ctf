"use strict";

// Keep the flag form intact; update only price text from authenticated quotes.
(() => {
  const prices = document.querySelectorAll("[data-challenge-points]");
  if (!prices.length) return;
  const status = document.querySelector("[data-price-status]");
  let pending = false;
  async function refresh() {
    if (pending || document.hidden) return;
    pending = true;
    try {
      const response = await fetch("/api/challenges/points", {
        credentials: "same-origin", cache: "no-store", redirect: "error"
      });
      if (!response.ok) throw new Error("Price update failed");
      const data = await response.json();
      const quotes = new Map(data.challenges.map(c => [c.id, c]));
      for (const element of prices) {
        const quote = quotes.get(element.dataset.challengePoints);
        if (!quote) continue;
        const solved = quote.points_awarded !== null;
        element.textContent = `${solved ? quote.points_awarded : quote.effective_points} pts${solved ? " earned" : ""}`;
      }
      for (const element of document.querySelectorAll("[data-challenge-gross]")) {
        const quote = quotes.get(element.dataset.challengeGross);
        if (quote) element.textContent = quote.current_points;
      }
      if (status) status.textContent = data.challenges.length
        ? "Prices updated. The value at submission time determines your award."
        : "Challenges are unavailable or the round has ended. Refresh the page to continue.";
    } catch (_) {
      if (status) status.textContent = "Live prices unavailable; refresh the page for current points.";
    } finally {
      pending = false;
    }
  }
  setInterval(refresh, 10000);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) refresh(); });
})();
