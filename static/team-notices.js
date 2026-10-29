(() => {
  const shown = new Set();
  let busy = false;
  async function poll() {
    if (busy) return;
    busy = true;
    try {
      const response = await fetch('/api/team/notices', {cache: 'no-store'});
      if (!response.ok || response.redirected) return;
      const data = await response.json();
      const container = document.getElementById('team-notices');
      if (!container) return;
      for (const notice of data.notices || []) {
        if (shown.has(notice.id)) continue;
        shown.add(notice.id);
        const banner = document.createElement('div');
        banner.className = `alert ${notice.kind === 'penalty' ? 'bad' : 'good'}`;
        banner.textContent = notice.message;
        container.appendChild(banner);
      }
    } catch (_) { /* A later poll retries temporary network failures. */ }
    finally { busy = false; }
  }
  poll();
  setInterval(poll, 2000);
})();
