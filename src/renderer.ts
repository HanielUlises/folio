/// <reference lib="dom" />
/// <reference lib="dom.iterable" />
'use strict';

// ════════════════════════════════════════════════════════════════════════════
//  Folio — renderer.ts
//  Frontend for the Tauri v2 PDF reader.
//
//  ALL PDF rendering, text extraction and thumbnail generation is done by
//  the Rust backend via Tauri commands.  This file contains ZERO pdf.js
//  calls.  Each opened tab has:
//    • A session_id returned by pdf_open()
//    • Lazily rendered pages (IntersectionObserver)
//    • A transparent text layer (absolute <span> elements) for selection
//    • A highlight layer (absolute <div> elements)
// ════════════════════════════════════════════════════════════════════════════

// ─── Types ───────────────────────────────────────────────────────────────────

interface Topic         { id: string; name: string; color: string; }
interface PdfEntry      { id: string; path: string; name: string; size: number; added: number; topicId: string | null; exists: boolean; }
interface HighlightRect { x: number; y: number; w: number; h: number; }
interface PdfHighlight  { id: string; page: number; rects: HighlightRect[]; color: string; }
interface AppData       { topics: Topic[]; pdfs: PdfEntry[]; highlights: Record<string, PdfHighlight[]>; }
interface OpenedFile    { path: string; name: string; size: number; added: number; }

interface TextSpan {
  text: string;
  x: number; y: number; w: number; h: number;
  font_size: number;
}

interface DocInfo {
  session_id: string;
  page_count: number;
}

interface TabMeta {
  id:         string;
  type:       'lib' | 'reader';
  pdfId?:     string;
  label:      string;
  topicColor: string | null;
}

interface ReaderState {
  sessionId:   string;
  pdfId:       string;
  pageCount:   number;
  currentPage: number;
  zoom:        number;             // CSS zoom factor (1.0 = 100 %)
  hlColor:     string;
  selData:     SelectionData | null;
  sMatches:    HTMLElement[];
  sIdx:        number;
  io:          IntersectionObserver | null;
  // page sizes in points (lazy, filled on first render)
  pageSizes:   Array<[number, number] | null>;
}

interface SelectionData {
  pageNum: number;       // 1-based
  rects:   HighlightRect[];
}

// Tauri v2 invoke
declare const __TAURI__: {
  core:    { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> };
  event:   { listen<T>(event: string, handler: (e: { payload: T }) => void): Promise<() => void> };
};

const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  __TAURI__.core.invoke<T>(cmd, args);

// ─── Tiny DOM helpers ─────────────────────────────────────────────────────────
const $   = (id: string): HTMLElement | null => document.getElementById(id);
const gId = (): string => crypto.randomUUID?.() ?? Math.random().toString(36).slice(2, 11);
const fmtSz = (b: number): string =>
  b < 1024 ? b + 'B' : b < 1048576 ? (b / 1024).toFixed(0) + 'KB' : (b / 1048576).toFixed(1) + 'MB';
const fmtDt = (ts: number): string =>
  new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
const esc = (s: unknown): string =>
  String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
const svgI = (id: string, w = 14, h = 14): string =>
  `<svg width="${w}" height="${h}" style="flex-shrink:0;display:inline-block"><use href="#i-${id}"/></svg>`;

function toast(msg: string): void {
  const t = $('toast'); if (!t) return;
  t.textContent = msg; t.classList.add('show');
  setTimeout(() => t.classList.remove('show'), 2200);
}

// ─── Global state ─────────────────────────────────────────────────────────────
let appState: AppData       = { topics: [], pdfs: [], highlights: {} };
let activeTopic: string | null = null;
let viewMode: 'grid' | 'list' = 'grid';
let searchQ   = '';
let activeTabId               = '__lib__';
let ddPdf: PdfEntry | null    = null;
let pendingAssignId: string | null = null;
let tColor = '#d4a843';

const tabMeta     = new Map<string, TabMeta>();
const tabOrder: string[] = [];
const readers     = new Map<string, ReaderState>();
const coverCache  = new Map<string, string | null>();    // pdfId → data-url | null
const coverLoading = new Set<string>();

const COLORS        = ['#d4a843','#6eb5d4','#7dcf8c','#d46e6e','#b57dcf','#cf9c7d','#7db8cf','#cfcf7d','#cf7da8','#7dcfcf'];
const HL_COLORS_SP  = ['#f9e04b','#7de87d','#7dc3f9','#f97d7d','#d4a843'];
let defaultHlColor  = '#f9e04b';
let globalFontSize  = 14;

let hlModeActive    = false;
let hlModeTid: string | null = null;
let selTid: string | null    = null;
let settingsTid: string | null = null;

// ─── Bootstrap ───────────────────────────────────────────────────────────────

async function boot(): Promise<void> {
  appState            = await invoke<AppData>('get_data');
  appState.topics     = appState.topics    ?? [];
  appState.pdfs       = appState.pdfs      ?? [];
  appState.highlights = appState.highlights ?? {};

  tabMeta.set('__lib__', { id: '__lib__', type: 'lib', label: 'Library', topicColor: null });
  tabOrder.push('__lib__');
  renderTabs(); render();
  checkFiles();

  // Listen for drag-and-drop events emitted by the Rust backend
  await __TAURI__.event.listen<OpenedFile[]>('folio-drop', async (event) => {
    const files = event.payload;
    if (!files.length) return;
    const toOpen: PdfEntry[] = [];
    let added = 0;
    for (const f of files) {
      let pdf = appState.pdfs.find(p => p.path === f.path);
      if (!pdf) {
        pdf = {
          id: gId(), path: f.path, name: f.name,
          size: f.size, added: f.added,
          topicId: (activeTopic && activeTopic !== '__u') ? activeTopic : null,
          exists: true,
        };
        appState.pdfs.push(pdf); added++;
      }
      toOpen.push(pdf);
    }
    if (added > 0) { await save(); render(); toast(`Added ${added} PDF${added !== 1 ? 's' : ''}`); }
    toOpen.forEach(p => openPdfTab(p));
  });
}

async function save(): Promise<void> {
  await invoke<boolean>('save_data', { data: appState });
}

// ─── Library thumbnails ───────────────────────────────────────────────────────
//
// Thumbnails are rendered entirely in Rust (pdfium) and returned as data-URLs.

async function loadCover(pdf: PdfEntry): Promise<void> {
  if (coverCache.has(pdf.id) || coverLoading.has(pdf.id)) return;
  coverLoading.add(pdf.id);
  try {
    const dataUrl = await invoke<string>('pdf_thumbnail', { path: pdf.path, thumbWidth: 240 });
    coverCache.set(pdf.id, dataUrl);
    stampCover(pdf.id, dataUrl);
  } catch {
    coverCache.set(pdf.id, null);
    stampCover(pdf.id, null);
  } finally {
    coverLoading.delete(pdf.id);
  }
}

function stampCover(pdfId: string, url: string | null): void {
  const ph = document.getElementById('cph-' + pdfId);
  if (ph) {
    if (url) {
      const img = document.createElement('img'); img.src = url;
      ph.parentElement?.appendChild(img); ph.remove();
    } else {
      ph.innerHTML = `<svg width="26" height="26" opacity=".18"><use href="#i-doc"/></svg>`;
      ph.querySelector('.cspin')?.remove();
    }
  }
  const lt = document.getElementById('lt-' + pdfId);
  if (lt && url) { lt.innerHTML = ''; const img = document.createElement('img'); img.src = url; lt.appendChild(img); }
}

// ─── Tabs ─────────────────────────────────────────────────────────────────────

function renderTabs(): void {
  const bar = $('tabbar'); if (!bar) return;
  bar.querySelectorAll('.tab').forEach(el => el.remove());
  const addBtn = $('tab-add');
  tabOrder.forEach(tid => {
    const m = tabMeta.get(tid); if (!m) return;
    const el = document.createElement('div');
    el.className = 'tab' + (tid === activeTabId ? ' active' : '');
    if (m.type === 'lib') {
      el.innerHTML = `${svgI('home', 13, 13)}<span class="tab-lbl">Library</span>`;
    } else {
      el.innerHTML = `${m.topicColor
        ? `<span class="tab-dot" style="background:${m.topicColor}"></span>`
        : svgI('doc', 13, 13)
      }<span class="tab-lbl" title="${esc(m.label)}">${esc(m.label)}</span><button class="tab-x">${svgI('x', 9, 9)}</button>`;
    }
    el.addEventListener('click', e => {
      if ((e.target as HTMLElement).closest('.tab-x')) return;
      switchTab(tid);
    });
    if (m.type !== 'lib')
      el.querySelector('.tab-x')?.addEventListener('click', e => { e.stopPropagation(); closeTab(tid); });
    bar.insertBefore(el, addBtn);
  });
}

function switchTab(tid: string): void {
  activeTabId = tid;
  document.querySelectorAll('.page').forEach(p => p.classList.add('hide'));
  $('page-' + tid)?.classList.remove('hide');
  $('sidebar')?.classList.toggle('hidden', tid !== '__lib__');
  const m   = tabMeta.get(tid);
  const ctx = $('tbar-ctx');
  if (ctx) ctx.textContent = m ? (m.type === 'lib' ? 'Library' : m.label) : '';
  renderTabs();
  (document.querySelector('.tab.active') as HTMLElement | null)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

async function closeTab(tid: string): Promise<void> {
  const idx = tabOrder.indexOf(tid); if (idx < 0) return;
  tabOrder.splice(idx, 1); tabMeta.delete(tid);
  const r = readers.get(tid);
  if (r) {
    r.io?.disconnect();
    await invoke('pdf_close', { sessionId: r.sessionId }).catch(() => {});
  }
  readers.delete(tid);
  $('page-' + tid)?.remove();
  switchTab(tabOrder[Math.max(0, idx - 1)] ?? '__lib__');
}

function openPdfTab(pdf: PdfEntry): void {
  // Re-use existing tab if already open
  for (const [tid, m] of tabMeta) {
    if (m.pdfId === pdf.id) { switchTab(tid); return; }
  }
  const tid   = 'r_' + gId();
  const topic = appState.topics.find(t => t.id === pdf.topicId);
  tabMeta.set(tid, { id: tid, type: 'reader', pdfId: pdf.id, label: pdf.name, topicColor: topic?.color ?? null });
  tabOrder.push(tid);

  const page = document.createElement('div');
  page.id = 'page-' + tid; page.className = 'page hide reader-page';
  page.innerHTML = `
    <div class="rtb">
      <button class="rback" data-tid="${tid}">${svgI('back', 12, 12)} Library</button>
      <span class="rtitle">${esc(pdf.name)}</span>
      <button class="vb" data-a="prev" data-tid="${tid}">${svgI('prev', 12, 12)}</button>
      <span class="pind" id="pind-${tid}">— / —</span>
      <button class="vb" data-a="next" data-tid="${tid}">${svgI('next', 12, 12)}</button>
      <div class="vsep"></div>
      <button class="vb" data-a="zo"   data-tid="${tid}">${svgI('zout', 13, 13)}</button>
      <span class="zlbl" id="zlbl-${tid}">140%</span>
      <button class="vb" data-a="zi"   data-tid="${tid}">${svgI('zin', 13, 13)}</button>
      <div class="vsep"></div>
      <button class="vb" data-a="fs"     data-tid="${tid}" title="Find in document">Search…</button>
      <button class="vb hl-mode-btn" data-a="hlmode" data-tid="${tid}" title="Highlight mode">${svgI('hlmode', 14, 14)}</button>
      <div class="vsep"></div>
      <button class="vb" data-a="settings" data-tid="${tid}" title="Settings">${svgI('settings', 13, 13)}</button>
    </div>
    <div class="rfind" id="rfind-${tid}">
      <svg width="13" height="13" style="opacity:.35;flex-shrink:0"><use href="#i-search"/></svg>
      <input type="text" id="rsi-${tid}" placeholder="Find in document…">
      <span class="finfo" id="finfo-${tid}"></span>
      <button class="vb" data-a="sp" data-tid="${tid}">${svgI('prev', 11, 11)}</button>
      <button class="vb" data-a="sn" data-tid="${tid}">${svgI('next', 11, 11)}</button>
      <button class="vb" data-a="sc" data-tid="${tid}">${svgI('x', 11, 11)}</button>
    </div>
    <div class="pdfvp" id="vp-${tid}">
      <div class="pdf-pages-inner" id="vpi-${tid}">
        <div style="padding:40px;color:var(--t3);display:flex;align-items:center;gap:12px">
          <div class="cspin"></div> Loading…
        </div>
      </div>
    </div>`;

  $('pages')?.appendChild(page);

  page.querySelector('.rback')?.addEventListener('click', () => switchTab('__lib__'));
  page.querySelectorAll<HTMLElement>('[data-a]').forEach(btn => {
    btn.addEventListener('click', e => {
      const a   = (e.currentTarget as HTMLElement).dataset['a'];
      const tid = (e.currentTarget as HTMLElement).dataset['tid'] ?? '';
      if      (a === 'prev')     rPrev(tid);
      else if (a === 'next')     rNext(tid);
      else if (a === 'zi')       rZoom(tid, 0.2);
      else if (a === 'zo')       rZoom(tid, -0.2);
      else if (a === 'fs')       toggleFind(tid);
      else if (a === 'hlmode')   toggleHlMode(tid);
      else if (a === 'settings') toggleSettings(tid);
      else if (a === 'sp')       navFind(tid, -1);
      else if (a === 'sn')       navFind(tid, 1);
      else if (a === 'sc')       closeFind(tid);
    });
  });
  ($('rsi-' + tid) as HTMLInputElement | null)?.addEventListener('input', e =>
    doFind(tid, (e.target as HTMLInputElement).value));

  renderTabs(); switchTab(tid);
  loadReaderPdf(tid, pdf);
}

// ─── Reader — open & scaffold ─────────────────────────────────────────────────

async function loadReaderPdf(tid: string, pdf: PdfEntry): Promise<void> {
  const vpi = $('vpi-' + tid); if (!vpi) return;

  const exists = await invoke<boolean>('check_exists', { path: pdf.path });
  if (!exists) {
    vpi.innerHTML = `<div style="padding:30px;color:var(--danger);display:flex;align-items:center;gap:8px">${svgI('warn', 16, 16)} File not found: <span style="opacity:.5;font-size:11px">${esc(pdf.path)}</span></div>`;
    return;
  }

  try {
    // 1. Ask Rust to open the document and get back a session_id + page_count
    const info = await invoke<DocInfo>('pdf_open', { path: pdf.path });

    // 2. Store reader state
    const state: ReaderState = {
      sessionId:   info.session_id,
      pdfId:       pdf.id,
      pageCount:   info.page_count,
      currentPage: 1,
      zoom:        1.4,
      hlColor:     defaultHlColor,
      selData:     null,
      sMatches:    [],
      sIdx:        0,
      io:          null,
      pageSizes:   new Array(info.page_count).fill(null),
    };
    readers.set(tid, state);
    updateNav(tid);

    // 3. Build page placeholder elements (rendered lazily via IntersectionObserver)
    vpi.innerHTML = '';
    for (let n = 1; n <= info.page_count; n++) {
      const wrap = document.createElement('div');
      wrap.className    = 'pwrap';
      wrap.dataset['page'] = String(n);

      // We need at least a rough size for the placeholder so the scroll height
      // is correct before rendering.  Use A4 aspect ratio as default.
      // Actual size is filled in when the page is first rendered.
      const estW = Math.floor(595 * state.zoom);
      const estH = Math.floor(842 * state.zoom);
      wrap.style.width  = estW + 'px';
      wrap.style.height = estH + 'px';

      const ph = document.createElement('div'); ph.className = 'page-loading';
      ph.innerHTML = `<div class="cspin" style="width:16px;height:16px;border-width:1.5px"></div>`;
      wrap.appendChild(ph);

      const hl = document.createElement('div'); hl.className = 'hllayer'; hl.dataset['page'] = String(n);
      wrap.appendChild(hl);

      const tl = document.createElement('div'); tl.className = 'textLayer'; tl.dataset['page'] = String(n);
      wrap.appendChild(tl);

      vpi.appendChild(wrap);
    }

    restoreHl(tid, pdf.id);
    attachObserver(tid);

  } catch (err) {
    const v2 = $('vpi-' + tid);
    if (v2) v2.innerHTML = `<div style="padding:30px;color:var(--danger);display:flex;align-items:center;gap:8px">${svgI('warn', 16, 16)} ${esc(err instanceof Error ? err.message : String(err))}</div>`;
  }
}

// ─── Page rendering ───────────────────────────────────────────────────────────

async function renderPage(tid: string, wrap: HTMLElement, pageNum: number): Promise<void> {
  const r = readers.get(tid); if (!r) return;

  // Device pixel ratio for crisp HiDPI rendering
  const dpr   = window.devicePixelRatio ?? 1;
  const scale = r.zoom * dpr;

  try {
    // Ask Rust for the rendered page as base64 PNG
    const b64 = await invoke<string>('pdf_render_page', {
      sessionId:  r.sessionId,
      pageIndex:  pageNum - 1,   // Rust is 0-based
      scale,
    });

    // Get page dimensions for layout
    let [wPts, hPts] = r.pageSizes[pageNum - 1] ?? [595, 842];
    if (r.pageSizes[pageNum - 1] === null) {
      const sz = await invoke<[number, number]>('pdf_page_size', {
        sessionId:  r.sessionId,
        pageIndex:  pageNum - 1,
      });
      [wPts, hPts] = sz;
      r.pageSizes[pageNum - 1] = sz;
    }

    if (!readers.get(tid)) return; // tab closed while awaiting

    const cssW = Math.floor(wPts * r.zoom);
    const cssH = Math.floor(hPts * r.zoom);
    wrap.style.width  = cssW + 'px';
    wrap.style.height = cssH + 'px';

    // Replace placeholder canvas
    wrap.querySelectorAll('canvas').forEach(c => c.remove());
    const cv  = document.createElement('canvas');
    cv.width  = Math.floor(wPts * scale);
    cv.height = Math.floor(hPts * scale);
    cv.style.width  = cssW + 'px';
    cv.style.height = cssH + 'px';

    const ctx = cv.getContext('2d')!;
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload  = () => resolve();
      img.onerror = reject;
      img.src = 'data:image/png;base64,' + b64;
    });
    ctx.drawImage(img, 0, 0);

    wrap.insertBefore(cv, wrap.firstChild);
    wrap.querySelector('.page-loading')?.remove();

    // Render text layer for selection / search
    await renderTextLayer(tid, wrap, pageNum, r);

  } catch (err) {
    console.error('renderPage', pageNum, err);
    wrap.querySelector('.page-loading')?.remove();
    const errDiv = document.createElement('div');
    errDiv.style.cssText = 'padding:8px;color:var(--danger);font-size:11px';
    errDiv.textContent = 'Render failed: ' + String(err);
    wrap.appendChild(errDiv);
  }
}

async function renderTextLayer(
  tid: string,
  wrap: HTMLElement,
  pageNum: number,
  r: ReaderState,
): Promise<void> {
  const tl = wrap.querySelector<HTMLElement>('.textLayer'); if (!tl) return;
  tl.innerHTML = '';

  const [wPts, hPts] = r.pageSizes[pageNum - 1] ?? [595, 842];
  tl.style.width  = Math.floor(wPts * r.zoom) + 'px';
  tl.style.height = Math.floor(hPts * r.zoom) + 'px';

  try {
    const spans = await invoke<TextSpan[]>('pdf_text_layer', {
      sessionId:  r.sessionId,
      pageIndex:  pageNum - 1,
      scale:      r.zoom,
    });

    for (const sp of spans) {
      const el = document.createElement('span');
      el.textContent    = sp.text;
      el.style.left     = sp.x + 'px';
      el.style.top      = sp.y + 'px';
      el.style.width    = sp.w + 'px';
      el.style.height   = sp.h + 'px';
      el.style.fontSize = (sp.font_size * r.zoom) + 'px';
      // Scale text to exactly fill its bounding box width
      if (sp.w > 2 && sp.text.length > 0) {
        const measured = sp.text.length * sp.font_size * r.zoom * 0.6; // rough em estimate
        if (measured > 0) {
          const scaleX = sp.w / measured;
          el.style.transform = `scaleX(${Math.min(scaleX, 2).toFixed(3)})`;
        }
      }
      tl.appendChild(el);
    }
  } catch (err) {
    console.warn('text layer', pageNum, err);
  }
}

// ─── IntersectionObserver — lazy rendering ────────────────────────────────────

function attachObserver(tid: string): void {
  const r   = readers.get(tid); if (!r) return;
  const vpEl = $('vp-' + tid); if (!vpEl) return;
  const vpi  = $('vpi-' + tid); if (!vpi) return;

  r.io?.disconnect(); r.io = null;

  const io = new IntersectionObserver(entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const w = entry.target as HTMLElement;
      const n = parseInt(w.dataset['page'] ?? '0', 10);
      const rr = readers.get(tid);
      if (rr) { rr.currentPage = n; updateNav(tid); }
      if (!w.dataset['rendered']) {
        w.dataset['rendered'] = '1';
        renderPage(tid, w, n).catch(console.error);
      }
    }
  }, { root: vpEl, threshold: 0.01, rootMargin: '400px 0px' });

  vpi.querySelectorAll<HTMLElement>('.pwrap').forEach(w => io.observe(w));
  r.io = io;
}

async function reRenderAll(tid: string): Promise<void> {
  const r = readers.get(tid); if (!r) return;
  const vpi = $('vpi-' + tid); if (!vpi) return;

  r.io?.disconnect(); r.io = null;

  for (const wrap of vpi.querySelectorAll<HTMLElement>('.pwrap')) {
    wrap.dataset['rendered'] = '';
    wrap.querySelectorAll('canvas').forEach(c => c.remove());
    const tl = wrap.querySelector<HTMLElement>('.textLayer'); if (tl) tl.innerHTML = '';
    wrap.querySelector('.page-loading')?.remove();
    const ph = document.createElement('div'); ph.className = 'page-loading';
    ph.innerHTML = `<div class="cspin" style="width:16px;height:16px;border-width:1.5px"></div>`;
    wrap.insertBefore(ph, wrap.firstChild);
  }

  attachObserver(tid);
  restoreHl(tid, r.pdfId);
}

function updateNav(tid: string): void {
  const r = readers.get(tid); if (!r) return;
  const pind = $('pind-' + tid); if (pind) pind.textContent = `${r.currentPage} / ${r.pageCount}`;
  const zlbl = $('zlbl-' + tid); if (zlbl) zlbl.textContent = Math.round(r.zoom * 100) + '%';
  const pg   = $('page-' + tid); if (!pg) return;
  (pg.querySelector('[data-a="prev"]') as HTMLButtonElement | null)!.disabled = r.currentPage <= 1;
  (pg.querySelector('[data-a="next"]') as HTMLButtonElement | null)!.disabled = r.currentPage >= r.pageCount;
}

function scrollToPage(tid: string, n: number): void {
  $('vpi-' + tid)?.querySelector<HTMLElement>(`.pwrap[data-page="${n}"]`)
    ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}
function rPrev(tid: string): void { const r = readers.get(tid); if (r && r.currentPage > 1) scrollToPage(tid, r.currentPage - 1); }
function rNext(tid: string): void { const r = readers.get(tid); if (r && r.currentPage < r.pageCount) scrollToPage(tid, r.currentPage + 1); }
async function rZoom(tid: string, d: number): Promise<void> {
  const r = readers.get(tid); if (!r) return;
  r.zoom = Math.min(3.5, Math.max(0.4, +((r.zoom + d).toFixed(1))));
  updateNav(tid); await reRenderAll(tid);
}

// ─── Text selection & highlight popup ────────────────────────────────────────

document.addEventListener('mouseup', () => {
  setTimeout(() => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.toString().trim()) return;
    for (const [tid, meta] of tabMeta) {
      if (meta.type !== 'reader') continue;
      const vpi = $('vpi-' + tid); if (!vpi) continue;
      if (sel.anchorNode && vpi.contains(sel.anchorNode)) { onSel(tid); return; }
    }
  }, 10);
});

function onSel(tid: string): void {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || !sel.toString().trim()) { hideHlPopup(); return; }

  let pc: HTMLElement | null = null;
  let pn = 0;
  for (let ri = 0; ri < sel.rangeCount; ri++) {
    const rng = sel.getRangeAt(ri);
    const candidate = (rng.startContainer as Node).parentElement?.closest<HTMLElement>('.pwrap');
    if (candidate) { pc = candidate; pn = parseInt(pc.dataset['page'] ?? '0', 10); break; }
  }
  if (!pc) { hideHlPopup(); return; }

  const pwrapRect = pc.getBoundingClientRect();
  const rects: HighlightRect[] = [];

  for (let ri = 0; ri < sel.rangeCount; ri++) {
    const rng = sel.getRangeAt(ri);
    for (const cr of rng.getClientRects()) {
      if (cr.width < 1 || cr.height < 1) continue;
      const x = cr.left - pwrapRect.left;
      const y = cr.top  - pwrapRect.top;
      if (x + cr.width < 0 || y + cr.height < 0) continue;
      if (x > pwrapRect.width + 10 || y > pwrapRect.height + 10) continue;
      rects.push({ x, y, w: cr.width, h: cr.height });
    }
  }
  if (!rects.length) { hideHlPopup(); return; }

  selTid = tid;
  const r = readers.get(tid); if (r) r.selData = { pageNum: pn, rects };

  if (hlModeActive && hlModeTid === tid) { applyHighlight(); return; }

  const firstRange = sel.getRangeAt(0);
  const br = firstRange.getBoundingClientRect();
  const popup = $('hl-popup');
  if (popup) {
    popup.style.left = Math.max(8, Math.min(window.innerWidth - 290, br.left + br.width / 2 - 140)) + 'px';
    popup.style.top  = Math.max(8, br.top - 52) + 'px';
    popup.classList.add('show');
  }
}

function hideHlPopup(): void { $('hl-popup')?.classList.remove('show'); selTid = null; }

function toggleHlMode(tid: string): void {
  const pg  = $('page-' + tid); if (!pg) return;
  const btn = pg.querySelector<HTMLElement>('.hl-mode-btn'); if (!btn) return;
  hlModeActive = !hlModeActive; hlModeTid = hlModeActive ? tid : null;
  btn.classList.toggle('active', hlModeActive);
  toast(hlModeActive ? 'Highlight mode ON — select text to highlight instantly' : 'Highlight mode OFF');
}

$('hl-popup')?.querySelectorAll<HTMLElement>('.hlcb').forEach(btn => {
  btn.addEventListener('mousedown', e => e.preventDefault());
  btn.addEventListener('click', () => {
    $('hl-popup')?.querySelectorAll('.hlcb').forEach(b => b.classList.remove('sel'));
    btn.classList.add('sel');
    if (selTid) { const r = readers.get(selTid); if (r) r.hlColor = btn.dataset['c'] ?? r.hlColor; }
  });
  btn.addEventListener('dblclick', () => {
    $('hl-popup')?.querySelectorAll('.hlcb').forEach(b => b.classList.remove('sel'));
    btn.classList.add('sel');
    if (selTid) { const r = readers.get(selTid); if (r) r.hlColor = btn.dataset['c'] ?? r.hlColor; }
    applyHighlight();
  });
});
$('hl-apply')?.addEventListener('mousedown', e => e.preventDefault());
$('hl-apply')?.addEventListener('click', applyHighlight);
$('hl-dismiss')?.addEventListener('click', () => { window.getSelection()?.removeAllRanges(); hideHlPopup(); });

async function applyHighlight(): Promise<void> {
  const tid = selTid; if (!tid) return;
  const r   = readers.get(tid); if (!r?.selData) return;
  const { pageNum, rects } = r.selData;
  const pdfId = r.pdfId;
  if (!appState.highlights[pdfId]) appState.highlights[pdfId] = [];
  const hlId  = gId();
  const color = r.hlColor || '#f9e04b';
  const newHl: PdfHighlight = { id: hlId, page: pageNum, rects, color };
  appState.highlights[pdfId]!.push(newHl);
  await save();
  drawHl(tid, pdfId, newHl);
  window.getSelection()?.removeAllRanges();
  hideHlPopup();
}

function drawHl(tid: string, pdfId: string, hl: PdfHighlight): void {
  const layer = $('vpi-' + tid)?.querySelector<HTMLElement>(`.hllayer[data-page="${hl.page}"]`);
  if (!layer) return;
  hl.rects.forEach(rect => {
    const d = document.createElement('div');
    d.className = 'hlr'; d.dataset['hlId'] = hl.id;
    d.style.cssText = `left:${rect.x}px;top:${rect.y}px;width:${rect.w}px;height:${rect.h}px;background:${hl.color};`;
    d.title = 'Click to remove';
    d.addEventListener('click', async e => {
      e.stopPropagation();
      appState.highlights[pdfId] = (appState.highlights[pdfId] ?? []).filter(h => h.id !== hl.id);
      $('vpi-' + tid)?.querySelectorAll(`[data-hl-id="${hl.id}"]`).forEach(el => el.remove());
      await save();
    });
    layer.appendChild(d);
  });
}

function restoreHl(tid: string, pdfId: string): void {
  const hls = appState.highlights[pdfId]; if (!hls) return;
  hls.forEach(hl => drawHl(tid, pdfId, hl));
}

// ─── In-document find ─────────────────────────────────────────────────────────
//
// Search is handled purely on the frontend by scanning already-rendered text
// layer spans (fast, no extra Rust call for already-visible pages).
// For pages not yet rendered, the Rust pdf_search command is used.

function toggleFind(tid: string): void {
  const b = $('rfind-' + tid); if (!b) return;
  b.classList.toggle('show');
  if (b.classList.contains('show')) setTimeout(() => ($('rsi-' + tid) as HTMLInputElement | null)?.focus(), 50);
}
function closeFind(tid: string): void { $('rfind-' + tid)?.classList.remove('show'); clearFind(tid); }
function clearFind(tid: string): void {
  $('vpi-' + tid)?.querySelectorAll<HTMLElement>('.fhl').forEach(el => {
    el.style.background = ''; el.style.outline = ''; el.classList.remove('fhl');
  });
  const r = readers.get(tid); if (r) { r.sMatches = []; r.sIdx = 0; }
  const fi = $('finfo-' + tid); if (fi) fi.textContent = '';
}

function doFind(tid: string, q: string): void {
  clearFind(tid); if (!q || !readers.get(tid)) return;
  const ql = q.toLowerCase();
  const matches: HTMLElement[] = [];
  $('vpi-' + tid)?.querySelectorAll<HTMLElement>('.textLayer span').forEach(s => {
    if (s.textContent?.toLowerCase().includes(ql)) {
      s.style.background = 'rgba(249,224,75,.5)'; s.classList.add('fhl'); matches.push(s);
    }
  });
  const r = readers.get(tid); if (r) { r.sMatches = matches; r.sIdx = 0; }
  const fi = $('finfo-' + tid); if (fi) fi.textContent = matches.length ? `${matches.length} found` : 'No results';
  if (matches.length) navFind(tid, 1);
}

function navFind(tid: string, dir: number): void {
  const r = readers.get(tid); if (!r || !r.sMatches.length) return;
  r.sMatches[r.sIdx]?.style && (r.sMatches[r.sIdx]!.style.outline = '');
  r.sIdx = (r.sIdx + dir + r.sMatches.length) % r.sMatches.length;
  const el = r.sMatches[r.sIdx]; if (!el) return;
  el.style.outline = '2px solid var(--ac)';
  el.scrollIntoView({ behavior: 'smooth', block: 'center' });
  const fi = $('finfo-' + tid); if (fi) fi.textContent = `${r.sIdx + 1} / ${r.sMatches.length}`;
}

// ─── Settings panel ───────────────────────────────────────────────────────────

function buildSettingsSwatches(): void {
  const row = $('sp-hl-swatches'); if (!row) return;
  row.innerHTML = '';
  HL_COLORS_SP.forEach(c => {
    const sw = document.createElement('div');
    sw.className = 'sp-swatch' + (c === defaultHlColor ? ' sel' : '');
    sw.style.background = c;
    sw.addEventListener('click', () => {
      defaultHlColor = c;
      row.querySelectorAll('.sp-swatch').forEach(s => s.classList.remove('sel'));
      sw.classList.add('sel');
      if (settingsTid) { const r = readers.get(settingsTid); if (r) r.hlColor = c; }
      $('hl-popup')?.querySelectorAll<HTMLElement>('.hlcb').forEach(b =>
        b.classList.toggle('sel', b.dataset['c'] === c));
    });
    row.appendChild(sw);
  });
}

function toggleSettings(tid: string): void {
  settingsTid = tid;
  const panel = $('settings-panel'); if (!panel) return;
  const showing = panel.classList.contains('show');
  panel.classList.toggle('show', !showing);
  if (!showing) {
    buildSettingsSwatches();
    const r = readers.get(tid);
    const spzv = $('sp-zoom-val'); if (spzv) spzv.textContent = r ? Math.round(r.zoom * 100) + '%' : '—';
    const spfv = $('sp-font-val'); if (spfv) spfv.textContent = globalFontSize + 'px';
  }
}

$('sp-zi')?.addEventListener('click', async () => {
  if (!settingsTid) return; const r = readers.get(settingsTid); if (!r) return;
  r.zoom = Math.min(3.5, +((r.zoom + 0.2).toFixed(1))); updateNav(settingsTid);
  const spzv = $('sp-zoom-val'); if (spzv) spzv.textContent = Math.round(r.zoom * 100) + '%';
  await reRenderAll(settingsTid);
});
$('sp-zo')?.addEventListener('click', async () => {
  if (!settingsTid) return; const r = readers.get(settingsTid); if (!r) return;
  r.zoom = Math.max(0.4, +((r.zoom - 0.2).toFixed(1))); updateNav(settingsTid);
  const spzv = $('sp-zoom-val'); if (spzv) spzv.textContent = Math.round(r.zoom * 100) + '%';
  await reRenderAll(settingsTid);
});
$('sp-fd')?.addEventListener('click', () => {
  globalFontSize = Math.min(22, globalFontSize + 1);
  document.body.style.fontSize = globalFontSize + 'px';
  const spfv = $('sp-font-val'); if (spfv) spfv.textContent = globalFontSize + 'px';
});
$('sp-fu')?.addEventListener('click', () => {
  globalFontSize = Math.max(11, globalFontSize - 1);
  document.body.style.fontSize = globalFontSize + 'px';
  const spfv = $('sp-font-val'); if (spfv) spfv.textContent = globalFontSize + 'px';
});
document.addEventListener('click', e => {
  const t = e.target as HTMLElement;
  if (!t.closest('#settings-panel') && !t.closest('[data-a="settings"]'))
    $('settings-panel')?.classList.remove('show');
});

// ─── Library rendering ────────────────────────────────────────────────────────

function getFiltered(): PdfEntry[] {
  let p = appState.pdfs;
  if (activeTopic === '__u') p = p.filter(x => !x.topicId);
  else if (activeTopic)      p = p.filter(x => x.topicId === activeTopic);
  if (searchQ) { const q = searchQ.toLowerCase(); p = p.filter(x => x.name.toLowerCase().includes(q)); }
  return p;
}

function render(): void { renderSidebar(); renderContent(); }

function renderSidebar(): void {
  const list = $('topiclist'); if (!list) return;
  list.innerHTML = '';
  const row = (label: string, color: string, count: number, active: boolean, onClick: () => void, onDel?: () => void): void => {
    const d = document.createElement('div'); d.className = 'trow' + (active ? ' active' : '');
    d.innerHTML = `<span class="tc" style="background:${color}"></span><span class="tn">${label}</span><span class="tbadge">${count}</span>${onDel ? `<button class="tdel">${svgI('x', 10, 10)}</button>` : ''}`;
    d.onclick = e => { if ((e.target as HTMLElement).closest('.tdel')) return; onClick(); };
    if (onDel) d.querySelector('.tdel')?.addEventListener('click', e => { e.stopPropagation(); onDel(); });
    list.appendChild(d);
  };
  row('All PDFs','#5e5c59', appState.pdfs.length, activeTopic === null, () => { activeTopic = null; render(); });
  const u = appState.pdfs.filter(p => !p.topicId).length;
  if (u > 0) row('Unsorted','#3e3c39', u, activeTopic === '__u', () => { activeTopic = '__u'; render(); });
  appState.topics.forEach(t => {
    const c = appState.pdfs.filter(p => p.topicId === t.id).length;
    row(esc(t.name), t.color, c, activeTopic === t.id, () => { activeTopic = t.id; render(); }, () => delTopic(t.id));
  });
  const tot = appState.pdfs.length;
  const sbf = $('sbfoot'); if (sbf) sbf.innerHTML = `<strong>${tot}</strong> PDF${tot !== 1 ? 's' : ''} · <strong>${appState.topics.length}</strong> topic${appState.topics.length !== 1 ? 's' : ''}`;
}

function renderContent(): void {
  const pdfs  = getFiltered(); const empty = pdfs.length === 0;
  const emptyEl = $('empty'); if (emptyEl) emptyEl.style.display = empty ? 'flex' : 'none';
  const pgrid   = $('pgrid');  if (pgrid)  pgrid.style.display   = (!empty && viewMode === 'grid') ? 'grid' : 'none';
  const plist   = $('plist');  if (plist)  plist.style.display   = (!empty && viewMode === 'list') ? 'flex' : 'none';
  let title = 'All PDFs';
  if (activeTopic === '__u') title = 'Unsorted';
  else if (activeTopic) { const t = appState.topics.find(t => t.id === activeTopic); if (t) title = `<span class="ti" style="background:${t.color}"></span>${esc(t.name)}`; }
  const lt = $('libtitle'); if (lt) lt.innerHTML = title;
  if (empty) return;
  viewMode === 'grid' ? renderGrid(pdfs) : renderList(pdfs);
}

function renderGrid(pdfs: PdfEntry[]): void {
  const g = $('pgrid'); if (!g) return; g.innerHTML = '';
  pdfs.forEach(pdf => {
    const topic = appState.topics.find(t => t.id === pdf.topicId);
    const card  = document.createElement('div'); card.className = 'card';
    const fallback = `<svg width="26" height="26" opacity=".18"><use href="#i-doc"/></svg>${topic ? `<div style="width:20px;height:3px;border-radius:2px;background:${topic.color};opacity:.65;margin-top:6px"></div>` : ''}`;
    card.innerHTML = `
      ${!pdf.exists ? `<div class="miss-badge">${svgI('warn', 10, 10)} Missing</div>` : ''}
      <div class="cover"><div class="cover-ph" id="cph-${pdf.id}">${fallback}
        <div class="cspin" style="position:absolute;bottom:8px;right:8px;width:13px;height:13px;border-width:1.5px"></div>
      </div></div>
      <div class="card-body">
        <div class="card-name" title="${esc(pdf.name)}">${esc(pdf.name)}</div>
        <div class="card-meta">${fmtSz(pdf.size || 0)}${topic ? ` · <span style="color:${topic.color}">${esc(topic.name)}</span>` : ''}</div>
      </div>
      <button class="card-menu">${svgI('dots', 13, 13)}</button>`;
    card.addEventListener('click', e => { if ((e.target as HTMLElement).closest('.card-menu')) return; openPdfTab(pdf); });
    card.querySelector('.card-menu')?.addEventListener('click', e => { e.stopPropagation(); showDD(e.currentTarget as HTMLElement, pdf); });
    g.appendChild(card);
    if (coverCache.has(pdf.id)) stampCover(pdf.id, coverCache.get(pdf.id) ?? null);
    else loadCover(pdf);
  });
}

function renderList(pdfs: PdfEntry[]): void {
  const l = $('plist'); if (!l) return; l.innerHTML = '';
  pdfs.forEach(pdf => {
    const topic = appState.topics.find(t => t.id === pdf.topicId);
    const row   = document.createElement('div'); row.className = 'lrow';
    row.innerHTML = `
      <div class="lthumb" id="lt-${pdf.id}"><svg width="14" height="14" opacity=".3"><use href="#i-doc"/></svg></div>
      <span class="lname">${!pdf.exists ? `<span style="color:var(--danger);margin-right:4px">${svgI('warn', 11, 11)}</span>` : ''}${esc(pdf.name)}</span>
      ${topic ? `<span class="lpill" style="background:${topic.color}22;color:${topic.color}">${esc(topic.name)}</span>` : `<span class="lpill" style="color:var(--t3)">—</span>`}
      <span class="lsize">${fmtSz(pdf.size || 0)}</span>
      <span class="ldate">${fmtDt(pdf.added)}</span>
      <button class="lmenu">${svgI('dots', 13, 13)}</button>`;
    row.addEventListener('click', e => { if ((e.target as HTMLElement).closest('.lmenu')) return; openPdfTab(pdf); });
    row.querySelector('.lmenu')?.addEventListener('click', e => { e.stopPropagation(); showDD(e.currentTarget as HTMLElement, pdf); });
    l.appendChild(row);
    if (coverCache.has(pdf.id)) stampCover(pdf.id, coverCache.get(pdf.id) ?? null);
    else loadCover(pdf);
  });
}

// ─── Import / open ────────────────────────────────────────────────────────────

$('btn-import')?.addEventListener('click', importPdfs);

async function importPdfs(): Promise<void> {
  const files = await invoke<OpenedFile[]>('open_pdf_dialog');
  if (!files.length) return;
  let added = 0;
  for (const f of files) {
    if (appState.pdfs.find(p => p.path === f.path)) continue;
    appState.pdfs.push({
      id: gId(), path: f.path, name: f.name, size: f.size, added: f.added,
      topicId: (activeTopic && activeTopic !== '__u') ? activeTopic : null,
      exists: true,
    });
    added++;
  }
  if (added > 0) { await save(); toast(`Added ${added} PDF${added !== 1 ? 's' : ''}`); render(); }
  else toast('Already in library');
}

$('tab-add')?.addEventListener('click', async () => {
  const files = await invoke<OpenedFile[]>('open_pdf_dialog');
  if (!files.length) return;
  const toOpen: PdfEntry[] = [];
  for (const f of files) {
    let pdf = appState.pdfs.find(p => p.path === f.path);
    if (!pdf) {
      pdf = { id: gId(), path: f.path, name: f.name, size: f.size, added: f.added, topicId: null, exists: true };
      appState.pdfs.push(pdf);
    }
    toOpen.push(pdf);
  }
  await save(); render(); toOpen.forEach(p => openPdfTab(p));
});

// ─── Topic management ─────────────────────────────────────────────────────────

$('btn-nt')?.addEventListener('click', () => { pendingAssignId = null; openTopicModal(); });

function openTopicModal(): void {
  tColor = COLORS[Math.floor(Math.random() * COLORS.length)] ?? '#d4a843';
  const tn = $('tname') as HTMLInputElement | null; if (tn) tn.value = '';
  const cp = $('cpicker'); if (cp) {
    cp.innerHTML = '';
    COLORS.forEach(c => {
      const sw = document.createElement('div'); sw.className = 'csw' + (c === tColor ? ' sel' : ''); sw.style.background = c;
      sw.onclick = () => { tColor = c; cp.querySelectorAll('.csw').forEach(s => s.classList.remove('sel')); sw.classList.add('sel'); };
      cp.appendChild(sw);
    });
  }
  $('mtopic')?.classList.add('show');
  setTimeout(() => ($('tname') as HTMLInputElement | null)?.focus(), 100);
}

$('btn-ct')?.addEventListener('click', () => { $('mtopic')?.classList.remove('show'); pendingAssignId = null; });
$('btn-st')?.addEventListener('click', async () => {
  const tn = $('tname') as HTMLInputElement | null;
  const name = tn?.value.trim() ?? ''; if (!name) return;
  const nid = gId(); appState.topics.push({ id: nid, name, color: tColor });
  if (pendingAssignId) { const p = appState.pdfs.find(p => p.id === pendingAssignId); if (p) p.topicId = nid; pendingAssignId = null; }
  await save(); $('mtopic')?.classList.remove('show'); render(); toast(`Topic "${name}" created`);
});
($('tname') as HTMLInputElement | null)?.addEventListener('keydown', e => {
  if (e.key === 'Enter') ($('btn-st') as HTMLButtonElement | null)?.click();
  if (e.key === 'Escape') ($('btn-ct') as HTMLButtonElement | null)?.click();
});

async function delTopic(id: string): Promise<void> {
  appState.pdfs = appState.pdfs.map(p => p.topicId === id ? { ...p, topicId: null } : p);
  appState.topics = appState.topics.filter(t => t.id !== id);
  if (activeTopic === id) activeTopic = null;
  await save(); render(); toast('Topic deleted');
}

// ─── Context menu dropdown ────────────────────────────────────────────────────

function showDD(anchor: HTMLElement, pdf: PdfEntry): void {
  ddPdf = pdf; const menu = $('ddm'); if (!menu) return;
  const tItems = appState.topics.map(t =>
    `<div class="ddi" data-a="assign" data-tid="${t.id}"><span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:${t.color};flex-shrink:0"></span>${esc(t.name)}</div>`
  ).join('');
  menu.innerHTML = `
    <div class="ddi" data-a="open">${svgI('doc', 13, 13)} Open in Tab</div>
    <div class="ddsep"></div><div class="ddsec">Assign Topic</div>
    <div class="ddi" data-a="assign" data-tid=""><span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--t3);flex-shrink:0"></span>None (Unsorted)</div>
    ${tItems}
    <div class="ddi" data-a="nt">${svgI('plus', 12, 12)} Create new topic…</div>
    <div class="ddsep"></div>
    <div class="ddi" data-a="rename">${svgI('edit', 12, 12)} Rename</div>
    <div class="ddi danger" data-a="remove">${svgI('trash', 12, 12)} Remove from Library</div>`;
  const rect = anchor.getBoundingClientRect();
  menu.style.top  = (rect.bottom + 4) + 'px';
  menu.style.left = rect.left + 'px';
  menu.classList.add('show');
  requestAnimationFrame(() => {
    const mr = menu.getBoundingClientRect();
    if (mr.right  > window.innerWidth  - 8) menu.style.left = (window.innerWidth  - mr.width  - 8) + 'px';
    if (mr.bottom > window.innerHeight - 8) menu.style.top  = (rect.top - mr.height - 4) + 'px';
  });
}

document.addEventListener('click', e => {
  const t = e.target as HTMLElement;
  if (!t.closest('#ddm') && !t.closest('.card-menu') && !t.closest('.lmenu'))
    $('ddm')?.classList.remove('show');
  if (!t.closest('#hl-popup') && !t.closest('.textLayer') && !t.closest('.pwrap'))
    hideHlPopup();
});

$('ddm')?.addEventListener('click', async e => {
  const item = (e.target as HTMLElement).closest<HTMLElement>('[data-a]');
  if (!item || !ddPdf) return;
  $('ddm')?.classList.remove('show');
  const a = item.dataset['a'];
  if      (a === 'open')   openPdfTab(ddPdf);
  else if (a === 'assign') { ddPdf.topicId = item.dataset['tid'] || null; await save(); render(); toast(item.dataset['tid'] ? 'Topic assigned' : 'Moved to Unsorted'); }
  else if (a === 'nt')     { pendingAssignId = ddPdf.id; openTopicModal(); }
  else if (a === 'rename') { const n = prompt('Rename:', ddPdf.name); if (n?.trim()) { ddPdf.name = n.trim(); await save(); render(); } }
  else if (a === 'remove') removePdf(ddPdf.id);
});

async function removePdf(id: string): Promise<void> {
  appState.pdfs = appState.pdfs.filter(p => p.id !== id);
  delete appState.highlights[id];
  coverCache.delete(id);
  for (const [tid, m] of tabMeta) { if (m.pdfId === id) closeTab(tid); }
  await save(); render(); toast('Removed from library');
}

// ─── View toggle & search ─────────────────────────────────────────────────────

$('vg')?.addEventListener('click', () => { viewMode = 'grid'; $('vg')?.classList.add('active'); $('vl')?.classList.remove('active'); renderContent(); });
$('vl')?.addEventListener('click', () => { viewMode = 'list'; $('vl')?.classList.add('active'); $('vg')?.classList.remove('active'); renderContent(); });
($('sinput') as HTMLInputElement | null)?.addEventListener('input', e => { searchQ = (e.target as HTMLInputElement).value; renderContent(); });

// ─── File existence check ─────────────────────────────────────────────────────

async function checkFiles(): Promise<void> {
  let changed = false;
  for (const pdf of appState.pdfs) {
    const e = await invoke<boolean>('check_exists', { path: pdf.path });
    if (pdf.exists !== e) { pdf.exists = e; changed = true; }
  }
  if (changed) { await save(); render(); }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

if (typeof window.__TAURI__ !== 'undefined') {
  boot();
} else {
  window.addEventListener('tauri-ready', () => boot(), { once: true });
}
