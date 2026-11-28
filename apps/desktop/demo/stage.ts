// The launch video's stage (stage.html), driven step by step by record-launch.mjs through
// `window.scene`. Recording-only, never shipped.
import '../src/app.css';

const css = `
:root{color-scheme:light}
html,body{margin:0;width:1920px;height:1080px;overflow:clip;background:#e7ecdd}
#stage{position:relative;width:1920px;height:1080px;overflow:clip;font-family:'DM Sans',system-ui,sans-serif;
  background:radial-gradient(1200px 700px at 20% 10%,#f4efe2 0,#e3ead6 55%,#d5e0c6 100%);color:#1d2219}
.hidden{opacity:0;pointer-events:none}
#menubar{position:absolute;inset:0 0 auto 0;height:30px;display:flex;justify-content:space-between;align-items:center;
  padding:0 18px;font-size:14px;background:rgba(255,255,255,.55);backdrop-filter:blur(20px);z-index:5}
#menubar .left,#menubar .right{display:flex;gap:20px;align-items:center}
#menubar .apple{font-size:16px}
.tray{display:inline-flex;width:18px;height:18px;color:#1d2219}
.win{position:absolute;border-radius:14px;background:#fff;box-shadow:0 30px 80px rgba(30,40,20,.28),0 0 0 1px rgba(0,0,0,.08);overflow:hidden;
  transition:opacity .45s ease,transform .45s ease}
.lights{display:flex;gap:8px;padding:0 4px}
.lights i{width:12px;height:12px;border-radius:50%;background:#ff5f57;display:block}
.lights i:nth-child(2){background:#febc2e}.lights i:nth-child(3){background:#28c840}
#chrome{left:120px;top:62px;width:1680px;height:960px}
.tabbar{height:46px;background:#e8e8e3;display:flex;align-items:flex-end;gap:10px;padding:0 10px}
.tabbar .lights{align-self:center;margin-right:6px}
#tabs{display:flex;flex:1;min-width:0;gap:1px;height:38px;align-items:flex-end}
.tab{flex:1 1 180px;min-width:28px;max-width:220px;height:36px;display:flex;align-items:center;gap:7px;padding:0 10px;
  border-radius:9px 9px 0 0;font-size:13px;color:#333;white-space:nowrap;overflow:hidden;transition:max-width .5s ease,flex-basis .5s ease,padding .5s ease,opacity .4s ease}
.tab.active{background:#fff}
.tab.gone{max-width:0;flex-basis:0;padding:0;opacity:0}
.tab.fresh{animation:fresh 1.2s ease}
@keyframes fresh{0%{background:#ddeb9d}100%{background:transparent}}
.tab.active.fresh{animation:freshA 1.2s ease}
@keyframes freshA{0%{background:#ddeb9d}100%{background:#fff}}
.fav{flex:none;width:16px;height:16px;border-radius:4px;display:grid;place-items:center;font-size:10px;font-weight:600;color:#fff}
.tab span{overflow:hidden;text-overflow:ellipsis}
.toolbar{height:44px;display:flex;align-items:center;gap:12px;padding:0 14px;border-bottom:1px solid #eee}
.url{flex:1;height:30px;border-radius:15px;background:#f1f1ee;display:flex;align-items:center;padding:0 16px;font-size:14px;color:#444}
.ext{width:22px;height:22px;border-radius:6px}
.page{padding:56px 72px;background:#fafafa;height:100%}
.page h2{font-size:34px;margin:0 0 6px;font-weight:600}
.page p{margin:0 0 30px;color:#666;font-size:18px}
.sk{height:14px;border-radius:7px;background:#e5e5e5;margin:14px 0}
.card{border:1px solid #e3e3e3;border-radius:16px;background:#fff;padding:26px 30px;margin-bottom:24px}
.note{position:absolute;top:44px;right:22px;width:400px;display:flex;gap:14px;align-items:center;padding:14px 16px;border-radius:18px;
  background:rgba(250,250,248,.92);backdrop-filter:blur(30px);box-shadow:0 12px 40px rgba(0,0,0,.18),0 0 0 1px rgba(0,0,0,.06);
  transform:translateX(460px);transition:transform .55s cubic-bezier(.2,.8,.2,1);z-index:6}
.note.show{transform:none}
.note img{width:40px;height:40px;border-radius:10px}
.note .body{flex:1;display:flex;flex-direction:column;font-size:14px;line-height:1.35}
.note button{border:0;background:#e9e9e4;border-radius:10px;padding:8px 14px;font:500 14px 'DM Sans';color:#222}
.note button.pressed{background:#cfd6c4}
#panel-wrap{position:absolute;left:600px;top:190px;width:720px;height:520px;transition:opacity .25s ease,transform .25s ease;z-index:7}
#panel-wrap.hidden{transform:scale(.97)}
#panel{position:relative;width:720px;height:520px;border:0;background:transparent;color-scheme:light}
#panel-wrap::before{content:'';position:absolute;inset:16px;border-radius:28px;box-shadow:0 30px 80px rgba(30,40,20,.30),0 4px 14px rgba(30,40,20,.12)}
.app .titlebar{height:30px;display:flex;align-items:center;gap:12px;padding:0 12px;background:#f3efe4;font-size:13px;color:#555;border-bottom:1px solid #e6e0d0}
.app .titlebar span{position:absolute;left:0;right:0;text-align:center;pointer-events:none}
#library{left:320px;top:110px;width:1280px;height:850px}
#library iframe{width:1280px;height:820px;border:0}
#settings{left:510px;top:150px;width:900px;height:730px}
#settings iframe{width:900px;height:700px;border:0}
.app.hidden{transform:translateY(16px) scale(.985)}
.caption{position:absolute;left:50%;bottom:64px;transform:translateX(-50%);padding:16px 34px;border-radius:999px;background:rgba(247,243,232,.96);
  box-shadow:0 10px 40px rgba(40,50,20,.22);font:400 44px/1.15 'Newsreader',serif;color:#1d2219;white-space:nowrap;transition:opacity .35s ease;z-index:9}
.keys{position:absolute;left:50%;top:120px;transform:translateX(-50%);display:flex;gap:10px;transition:opacity .25s ease;z-index:9}
.keys b{min-width:64px;height:64px;padding:0 14px;border-radius:14px;background:rgba(29,34,25,.86);color:#fff;display:grid;place-items:center;font:500 30px 'DM Sans';box-shadow:0 8px 24px rgba(0,0,0,.25)}
.later{position:absolute;inset:0;display:grid;place-items:center;background:rgba(247,243,232,.94);font:400 64px 'Newsreader',serif;color:#1d2219;transition:opacity .5s ease;z-index:8}
.sample{position:absolute;right:18px;bottom:12px;font-size:13px;color:rgba(29,34,25,.45);z-index:9}
#cursor{position:absolute;left:0;top:0;transform:translate(1500px,900px);transition:transform .9s cubic-bezier(.3,.7,.2,1);z-index:10;filter:drop-shadow(0 2px 3px rgba(0,0,0,.3))}
#cursor.click{animation:click .3s ease}
@keyframes click{50%{scale:.82}}
.end{position:absolute;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;background:#f7f3e8;transition:opacity .6s ease;z-index:11}
.end img{width:180px;height:180px;border-radius:40px;box-shadow:0 20px 50px rgba(61,111,58,.35)}
.end h1{font:400 120px/1 'Newsreader',serif;margin:34px 0 8px}
.end .tag{font:400 40px 'Newsreader',serif;color:#4a5340;margin:0 0 34px}
.end .site{font:500 34px 'DM Sans';color:#3d6f3a;margin:0 0 14px}
.end .price{font:400 26px 'DM Sans';color:#5b6352;margin:0}
`;
document.head.insertAdjacentHTML('beforeend', `<style>${css}</style>`);

const leaf = (fill: string) =>
  `<svg viewBox="160 160 704 704" width="100%" height="100%" fill="${fill}"><mask id="tl"><g transform="rotate(-45 512 512)"><path d="M512 196 C 700 330 700 694 512 828 C 324 694 324 330 512 196 Z" fill="#fff"/><path d="M512 270 L512 900" stroke="#000" stroke-width="40" stroke-linecap="round"/></g></mask><rect x="160" y="160" width="704" height="704" mask="url(#tl)"/></svg>`;
document.getElementById('tray')!.innerHTML = leaf('currentColor');

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const COLORS = [
  '#3d6f3a',
  '#5b6bd6',
  '#c2563c',
  '#2f8f83',
  '#8a5bd6',
  '#d08a1e',
  '#3b82c4',
  '#b04a7a',
];

interface Tab {
  title: string;
  host: string;
  el: HTMLElement;
}
const tabs: Tab[] = [];
function makeTab(title: string, host: string, i: number): Tab {
  const el = document.createElement('div');
  el.className = 'tab';
  el.innerHTML = `<i class="fav" style="background:${COLORS[i % COLORS.length]}">${title[0]}</i><span>${title}</span>`;
  return { title, host, el };
}

// 22 tabs: the Lisbon trip and old reading among everyday work.
const OPEN: [string, string][] = [
  ['Pull requests · acme/web', 'github.com/acme/web/pulls'],
  ['Onboarding flow v3 – Figma', 'figma.com'],
  ['DEC-214 Popup search ranking – Linear', 'linear.app'],
  ['Google Calendar – Week of 24 November', 'calendar.google.com'],
  ['Inbox (3) – Gmail', 'mail.google.com'],
  ['Tram 28: Lisbon’s most famous ride – Visit Lisboa', 'visitlisboa.com/tram-28'],
  ['Q3 planning notes – Google Docs', 'docs.google.com'],
  ['Lisbon restaurants in Alfama – Tripadvisor', 'tripadvisor.com'],
  ['Reading list – Notion', 'notion.so'],
  ['How do I cancel a fetch request? – Stack Overflow', 'stackoverflow.com'],
  ['Lisbon apartments, 12–16 Dec – Booking.com', 'booking.com'],
  ['Rust async, explained – YouTube', 'youtube.com'],
  ['Using the Fetch API – MDN', 'developer.mozilla.org'],
  ['The 30 best things to do in Lisbon – Time Out', 'timeout.com'],
  ['Show HN: A local-first tab archive', 'news.ycombinator.com'],
  ['Azulejo – Wikipedia', 'en.wikipedia.org'],
  ['Design QA checklist – Notion', 'notion.so'],
  ['Release v2.3 · acme/web', 'github.com'],
  ['Weekly sync notes – Google Docs', 'docs.google.com'],
  ['Flights to Lisbon – Google Flights', 'google.com/travel'],
  ['Pricing – Linear', 'linear.app'],
  ['CSS :has() selector – MDN', 'developer.mozilla.org'],
];
// What the sweep archives (the panel finds the same titles in the archive).
const SWEPT = [5, 6, 7, 9, 10, 12, 13, 14, 15];

function render() {
  $('tabs').replaceChildren(...tabs.map((t) => t.el));
}
function activate(t: Tab) {
  for (const o of tabs) o.el.classList.toggle('active', o === t);
  $('url').textContent = t.host;
  pageFor(t.title);
}
function pageFor(title: string) {
  const lisbon = /Lisbon|Tram 28|Alfama|Azulejo/.test(title);
  const head = lisbon ? title.split(' – ')[0] : 'Pull requests';
  const sub = lisbon ? 'Restored by Decay, right where it was' : 'Open · 12 pull requests';
  $('page').innerHTML = `<h2>${head}</h2><p>${sub}</p>
    <div class="card"><div class="sk" style="width:38%"></div><div class="sk" style="width:82%"></div><div class="sk" style="width:70%"></div><div class="sk" style="width:76%"></div></div>
    <div style="display:flex;gap:24px"><div class="card" style="flex:1"><div class="sk" style="width:40%"></div><div class="sk" style="width:80%"></div></div><div class="card" style="flex:1"><div class="sk" style="width:46%"></div><div class="sk" style="width:72%"></div></div></div>`;
}

let cursorAt = { x: 1500, y: 900 };
async function moveTo(x: number, y: number, ms = 900) {
  const c = $('cursor');
  c.style.transitionDuration = `${ms}ms`;
  c.style.transform = `translate(${x}px,${y}px)`;
  cursorAt = { x, y };
  await sleep(ms);
}
async function click() {
  const c = $('cursor');
  c.classList.remove('click');
  void c.getBoundingClientRect();
  c.classList.add('click');
  await sleep(300);
}

let onMessage: ((d: any) => void) | null = null;
window.addEventListener('message', (e) => {
  if (e.data?.decay) onMessage?.(e.data);
});

const show = (id: string, on = true) => $(id).classList.toggle('hidden', !on);

const scene = {
  async setup() {
    OPEN.forEach(([title, host], i) => tabs.push(makeTab(title, host, i)));
    render();
    activate(tabs[0]);
    $<HTMLIFrameElement>('panel').src = './index.html?view=panel';
    $<HTMLIFrameElement>('library-frame').src = './index.html?view=library#/';
    $<HTMLIFrameElement>('settings-frame').src = './index.html?view=settings#/protection';
    await sleep(2500);
  },
  async caption(text: string | null) {
    const c = $('caption');
    if (!text) return show('caption', false);
    if (!c.classList.contains('hidden')) {
      show('caption', false);
      await sleep(300);
    }
    c.textContent = text;
    show('caption');
  },
  async keys(list: string[] | null) {
    const k = $('keys');
    if (!list) return show('keys', false);
    k.innerHTML = list.map((x) => `<b>${x}</b>`).join('');
    show('keys');
  },
  async scrollTabs() {
    await moveTo(700, 80, 900);
    for (const i of [3, 8, 12, 16]) {
      await moveTo(170 + i * 72, 82, 350);
    }
  },
  async sweep() {
    $('note-text').textContent = 'Archived 9 tabs you haven’t used in days';
    $('note').classList.add('show');
    await sleep(500);
    for (const i of SWEPT) {
      tabs[i].el.classList.add('gone');
      await sleep(90);
    }
  },
  async undo() {
    const b = $('undo').getBoundingClientRect();
    await moveTo(b.left + b.width / 2 - 6, b.top + b.height / 2 - 4, 1000);
    await click();
    $('undo').classList.add('pressed');
    for (const i of SWEPT) {
      tabs[i].el.classList.remove('gone');
      tabs[i].el.classList.add('fresh');
    }
    await sleep(400);
    $('note').classList.remove('show');
    $('undo').classList.remove('pressed');
  },
  async later() {
    show('later');
    await sleep(700);
    for (const i of SWEPT) {
      tabs[i].el.classList.remove('fresh');
      tabs[i].el.remove();
    }
    tabs.splice(0, tabs.length, ...tabs.filter((_, i) => !SWEPT.includes(i)));
    render();
    activate(tabs[0]);
    await moveTo(1500, 1000, 10);
    await sleep(900);
    show('later', false);
    await sleep(500);
  },
  async openPanel() {
    $<HTMLIFrameElement>('panel').contentWindow?.postMessage(
      { decay: true, type: 'panel-open' },
      '*',
    );
    show('panel-wrap');
    await sleep(350);
  },
  /** Resolves when the panel restored something (its tabs are added and shown). */
  waitRestore() {
    return new Promise<void>((resolve) => {
      onMessage = (d) => {
        if (d.type === 'restore') {
          const fresh = (d.titles as string[]).filter(
            (title) => !tabs.some((t) => t.title === title),
          );
          const added = fresh.map((title, i) => {
            const t = makeTab(
              title,
              OPEN.find(([o]) => o.startsWith(title.split(' – ')[0]))?.[1] ?? 'example.com',
              5 + i,
            );
            t.el.classList.add('fresh');
            tabs.push(t);
            return t;
          });
          render();
          if (added[0]) activate(added[0]);
        }
        if (d.type === 'panel-hidden') {
          show('panel-wrap', false);
          onMessage = null;
          resolve();
        }
      };
    });
  },
  async library(on = true) {
    show('library', on);
    await sleep(500);
  },
  /** A point inside the Library frame (its own coordinates) on the stage. */
  libraryPoint(x: number, y: number) {
    const r = $('library-frame').getBoundingClientRect();
    return { x: r.left + x, y: r.top + y };
  },
  settingsPoint(x: number, y: number) {
    const r = $('settings-frame').getBoundingClientRect();
    return { x: r.left + x, y: r.top + y };
  },
  moveTo,
  click,
  async settings(on = true) {
    show('settings', on);
    await sleep(500);
  },
  async end() {
    show('caption', false);
    show('keys', false);
    show('end');
    await sleep(600);
  },
  cursorAt: () => cursorAt,
};
(window as unknown as { scene: typeof scene }).scene = scene;
