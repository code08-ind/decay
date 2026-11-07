// Recording-only page (not shipped): live tab strip from chrome.tabs + a Decay page in a frame.
const strip = document.getElementById('tabs');
const shown = new Map();
let selfId;
chrome.tabs.getCurrent((t) => { selfId = t?.id; render(); });
async function render() {
  const tabs = (await chrome.tabs.query({ windowType: "normal" })).filter((t) => t.id !== selfId);
  const ids = new Set(tabs.map((t) => t.id));
  for (const [id, el] of shown) if (!ids.has(id)) { el.classList.add('gone'); shown.delete(id); setTimeout(() => el.remove(), 600); }
  for (const t of tabs) {
    let el = shown.get(t.id);
    const title = t.title || t.url;
    if (!el) {
      el = document.createElement('div'); el.className = 'tab'; strip.append(el); shown.set(t.id, el);
      if (window.ready) { el.classList.add('new'); setTimeout(() => el.classList.remove('new'), 2500); }
    }
    el.innerHTML = `<i>${(new URL(t.url).hostname.replace(/^www\./, '')[0] || '?').toUpperCase()}</i><span class="t"></span>`;
    el.querySelector('.t').textContent = title;
  }
}
for (const e of ['onCreated', 'onRemoved', 'onUpdated', 'onMoved']) chrome.tabs[e].addListener(() => render());
window.show = (page, w, h) => { const f = document.getElementById('frame'); f.style.width = `${w}px`; f.style.height = `${h}px`; document.getElementById('view').src = page; };
window.say = (step, title, body, note = '') => { for (const [k, v] of Object.entries({ step, title, body, note })) document.getElementById(k).textContent = v; };
