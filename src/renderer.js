/// <reference lib="dom" />
/// <reference lib="dom.iterable" />
'use strict';
const invoke = (cmd, args) => __TAURI__.core.invoke(cmd, args);
pdfjsLib.GlobalWorkerOptions.workerSrc =
    'https://cdnjs.cloudflare.com/ajax/libs/pdf.js/3.11.174/pdf.worker.min.js';
const svgI = (id, w = 14, h = 14) => `<svg width="${w}" height="${h}" style="flex-shrink:0;display:inline-block"><use href="#i-${id}"/></svg>`;
const $ = (id) => document.getElementById(id);
const gId = () => Math.random().toString(36).slice(2, 11);
const fmtSz = (b) => b < 1024 ? b + 'B' : b < 1048576 ? (b / 1024).toFixed(0) + 'KB' : (b / 1048576).toFixed(1) + 'MB';
const fmtDt = (ts) => new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
function toast(msg) {
    const t = $('toast');
    if (!t)
        return;
    t.textContent = msg;
    t.classList.add('show');
    setTimeout(() => t.classList.remove('show'), 2200);
}
let appState = { topics: [], pdfs: [], highlights: {} };
let activeTopic = null;
let viewMode = 'grid';
let searchQ = '';
let activeTabId = '__lib__';
let ddPdf = null;
let pendingAssignId = null;
let tColor = '#d4a843';
const tabMeta = new Map();
const tabOrder = [];
const readers = new Map();
const coverCache = new Map();
const coverLoading = new Set();
const COLORS = ['#d4a843', '#6eb5d4', '#7dcf8c', '#d46e6e', '#b57dcf', '#cf9c7d', '#7db8cf', '#cfcf7d', '#cf7da8', '#7dcfcf'];
const HL_COLORS_SP = ['#f9e04b', '#7de87d', '#7dc3f9', '#f97d7d', '#d4a843'];
let defaultHlColor = '#f9e04b';
let globalFontSize = 14;
let hlModeActive = false;
let hlModeTid = null;
let selTid = null;
let settingsTid = null;
async function boot() {
    appState = await invoke('get_data');
    appState.topics = appState.topics ?? [];
    appState.pdfs = appState.pdfs ?? [];
    appState.highlights = appState.highlights ?? {};
    tabMeta.set('__lib__', { id: '__lib__', type: 'lib', label: 'Library', topicColor: null });
    tabOrder.push('__lib__');
    renderTabs();
    render();
    checkFiles();
}
async function save() { await invoke('save_data', { data: appState }); }
async function loadCover(pdf) {
    if (coverCache.has(pdf.id) || coverLoading.has(pdf.id))
        return;
    coverLoading.add(pdf.id);
    try {
        const url = await invoke('get_folio_url', { path: pdf.path });
        const doc = await pdfjsLib.getDocument({ url, disableAutoFetch: false, disableStream: false }).promise;
        const page = await doc.getPage(1);
        const scale = 240 / page.getViewport({ scale: 1 }).width;
        const vp = page.getViewport({ scale });
        const cv = document.createElement('canvas');
        cv.width = Math.floor(vp.width);
        cv.height = Math.floor(vp.height);
        const ctx = cv.getContext('2d');
        if (ctx)
            await page.render({ canvasContext: ctx, viewport: vp }).promise;
        doc.destroy();
        const dataUrl = cv.toDataURL('image/jpeg', 0.82);
        coverCache.set(pdf.id, dataUrl);
        stampCover(pdf.id, dataUrl);
    }
    catch {
        coverCache.set(pdf.id, null);
        stampCover(pdf.id, null);
    }
    finally {
        coverLoading.delete(pdf.id);
    }
}
function stampCover(pdfId, url) {
    const ph = document.getElementById('cph-' + pdfId);
    if (ph) {
        if (url) {
            const img = document.createElement('img');
            img.src = url;
            ph.parentElement?.appendChild(img);
            ph.remove();
        }
        else {
            ph.innerHTML = `<svg width="26" height="26" opacity=".18"><use href="#i-doc"/></svg>`;
            ph.querySelector('.cspin')?.remove();
        }
    }
    const lt = document.getElementById('lt-' + pdfId);
    if (lt && url) {
        lt.innerHTML = '';
        const img = document.createElement('img');
        img.src = url;
        lt.appendChild(img);
    }
}
function renderTabs() {
    const bar = $('tabbar');
    if (!bar)
        return;
    bar.querySelectorAll('.tab').forEach(el => el.remove());
    const addBtn = $('tab-add');
    tabOrder.forEach(tid => {
        const m = tabMeta.get(tid);
        if (!m)
            return;
        const el = document.createElement('div');
        el.className = 'tab' + (tid === activeTabId ? ' active' : '');
        if (m.type === 'lib') {
            el.innerHTML = `${svgI('home', 13, 13)}<span class="tab-lbl">Library</span>`;
        }
        else {
            el.innerHTML = `${m.topicColor
                ? `<span class="tab-dot" style="background:${m.topicColor}"></span>`
                : svgI('doc', 13, 13)}<span class="tab-lbl" title="${esc(m.label)}">${esc(m.label)}</span><button class="tab-x">${svgI('x', 9, 9)}</button>`;
        }
        el.addEventListener('click', e => { if (e.target.closest('.tab-x'))
            return; switchTab(tid); });
        if (m.type !== 'lib')
            el.querySelector('.tab-x')?.addEventListener('click', e => { e.stopPropagation(); closeTab(tid); });
        bar.insertBefore(el, addBtn);
    });
}
function switchTab(tid) {
    activeTabId = tid;
    document.querySelectorAll('.page').forEach(p => p.classList.add('hide'));
    $('page-' + tid)?.classList.remove('hide');
    $('sidebar')?.classList.toggle('hidden', tid !== '__lib__');
    const m = tabMeta.get(tid);
    const ctx = $('tbar-ctx');
    if (ctx)
        ctx.textContent = m ? (m.type === 'lib' ? 'Library' : m.label) : '';
    renderTabs();
    document.querySelector('.tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}
function closeTab(tid) {
    const idx = tabOrder.indexOf(tid);
    if (idx < 0)
        return;
    tabOrder.splice(idx, 1);
    tabMeta.delete(tid);
    const r = readers.get(tid);
    r?.io?.disconnect();
    r?.pdfDoc?.destroy();
    readers.delete(tid);
    $('page-' + tid)?.remove();
    switchTab(tabOrder[Math.max(0, idx - 1)] ?? '__lib__');
}
function openPdfTab(pdf) {
    for (const [tid, m] of tabMeta) {
        if (m.pdfId === pdf.id) {
            switchTab(tid);
            return;
        }
    }
    const tid = 'r_' + gId();
    const topic = appState.topics.find(t => t.id === pdf.topicId);
    tabMeta.set(tid, { id: tid, type: 'reader', pdfId: pdf.id, label: pdf.name, topicColor: topic?.color ?? null });
    tabOrder.push(tid);
    const page = document.createElement('div');
    page.id = 'page-' + tid;
    page.className = 'page hide reader-page';
    page.innerHTML = `
    <div class="rtb">
      <button class="rback" data-tid="${tid}">${svgI('back', 12, 12)} Library</button>
      <span class="rtitle">${esc(pdf.name)}</span>
      <button class="vb" data-a="prev" data-tid="${tid}">${svgI('prev', 12, 12)}</button>
      <span class="pind" id="pind-${tid}">— / —</span>
      <button class="vb" data-a="next" data-tid="${tid}">${svgI('next', 12, 12)}</button>
      <div class="vsep"></div>
      <button class="vb" data-a="zo" data-tid="${tid}">${svgI('zout', 13, 13)}</button>
      <span class="zlbl" id="zlbl-${tid}">140%</span>
      <button class="vb" data-a="zi" data-tid="${tid}">${svgI('zin', 13, 13)}</button>
      <div class="vsep"></div>
      <button class="vb" data-a="fs" data-tid="${tid}" title="Find in document">Search…</button>
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
    page.querySelectorAll('[data-a]').forEach(btn => {
        btn.addEventListener('click', e => {
            const a = e.currentTarget.dataset['a'];
            const t = e.currentTarget.dataset['tid'] ?? '';
            if (a === 'prev')
                rPrev(t);
            else if (a === 'next')
                rNext(t);
            else if (a === 'zi')
                rZoom(t, 0.2);
            else if (a === 'zo')
                rZoom(t, -0.2);
            else if (a === 'fs')
                toggleFind(t);
            else if (a === 'hlmode')
                toggleHlMode(t);
            else if (a === 'settings')
                toggleSettings(t);
            else if (a === 'sp')
                navFind(t, -1);
            else if (a === 'sn')
                navFind(t, 1);
            else if (a === 'sc')
                closeFind(t);
        });
    });
    $('rsi-' + tid)?.addEventListener('input', e => doFind(tid, e.target.value));
    renderTabs();
    switchTab(tid);
    loadReaderPdf(tid, pdf);
}
async function loadReaderPdf(tid, pdf) {
    const vpi = $('vpi-' + tid);
    if (!vpi)
        return;
    const exists = await invoke('check_exists', { path: pdf.path });
    if (!exists) {
        vpi.innerHTML = `<div style="padding:30px;color:var(--danger);display:flex;align-items:center;gap:8px">${svgI('warn', 16, 16)} File not found: <span style="opacity:.5;font-size:11px">${esc(pdf.path)}</span></div>`;
        return;
    }
    try {
        const folioUrl = await invoke('get_folio_url', { path: pdf.path });
        const pdfDoc = await pdfjsLib.getDocument({
            url: folioUrl, disableAutoFetch: false, disableStream: false,
            cMapUrl: 'https://cdnjs.cloudflare.com/ajax/libs/pdf.js/3.11.174/cmaps/', cMapPacked: true,
        }).promise;
        const pageCount = pdfDoc.numPages;
        readers.set(tid, { pdfDoc, pdfId: pdf.id, pageCount, currentPage: 1, zoom: 1.4, hlColor: defaultHlColor, selData: null, sMatches: [], sIdx: 0, io: null });
        updateNav(tid);
        vpi.innerHTML = '';
        for (let n = 1; n <= pageCount; n++) {
            const pg = await pdfDoc.getPage(n);
            const r = readers.get(tid);
            if (!r)
                return;
            const vp = pg.getViewport({ scale: r.zoom });
            const wrap = document.createElement('div');
            wrap.className = 'pwrap';
            wrap.dataset['page'] = String(n);
            wrap.style.width = Math.floor(vp.width) + 'px';
            wrap.style.height = Math.floor(vp.height) + 'px';
            const ph = document.createElement('div');
            ph.className = 'page-loading';
            ph.innerHTML = `<div class="cspin" style="width:16px;height:16px;border-width:1.5px"></div>`;
            wrap.appendChild(ph);
            const hl = document.createElement('div');
            hl.className = 'hllayer';
            hl.dataset['page'] = String(n);
            wrap.appendChild(hl);
            const tl = document.createElement('div');
            tl.className = 'textLayer';
            tl.dataset['page'] = String(n);
            wrap.appendChild(tl);
            vpi.appendChild(wrap);
        }
        restoreHl(tid, pdf.id);
        attachObserver(tid);
    }
    catch (e) {
        const v2 = $('vpi-' + tid);
        if (v2)
            v2.innerHTML = `<div style="padding:30px;color:var(--danger);display:flex;align-items:center;gap:8px">${svgI('warn', 16, 16)} ${esc(e instanceof Error ? e.message : String(e))}</div>`;
    }
}
async function renderPage(tid, wrap, pageNum) {
    const r = readers.get(tid);
    if (!r)
        return;
    const page = await r.pdfDoc.getPage(pageNum);
    if (!readers.get(tid))
        return;
    const vp = page.getViewport({ scale: r.zoom });
    const cv = document.createElement('canvas');
    cv.width = Math.floor(vp.width);
    cv.height = Math.floor(vp.height);
    const ctx = cv.getContext('2d');
    if (ctx)
        await page.render({ canvasContext: ctx, viewport: vp }).promise;
    if (!readers.get(tid))
        return;
    wrap.insertBefore(cv, wrap.firstChild);
    wrap.querySelector('.page-loading')?.remove();
    await renderTextLayer(wrap, page, vp);
}
async function renderTextLayer(wrap, page, vp) {
    const tl = wrap.querySelector('.textLayer');
    if (!tl)
        return;
    tl.innerHTML = '';
    tl.style.width = Math.floor(vp.width) + 'px';
    tl.style.height = Math.floor(vp.height) + 'px';
    try {
        const tc = await page.getTextContent();
        const task = pdfjsLib.renderTextLayer({
            textContentSource: tc,
            container: tl,
            viewport: vp,
            textDivs: [],
        });
        if (task && typeof task.promise?.then === 'function') {
            await task.promise;
        }
        else if (task && typeof task.then === 'function') {
            await task;
        }
    }
    catch { }
}
function attachObserver(tid) {
    const r = readers.get(tid);
    if (!r)
        return;
    const vpEl = $('vp-' + tid);
    if (!vpEl)
        return;
    const vpi = $('vpi-' + tid);
    if (!vpi)
        return;
    r.io?.disconnect();
    r.io = null;
    const io = new IntersectionObserver(entries => {
        for (const entry of entries) {
            if (!entry.isIntersecting)
                continue;
            const w = entry.target;
            const n = parseInt(w.dataset['page'] ?? '0', 10);
            const rr = readers.get(tid);
            if (rr) {
                rr.currentPage = n;
                updateNav(tid);
            }
            if (!w.dataset['rendered']) {
                w.dataset['rendered'] = '1';
                renderPage(tid, w, n).catch(() => { });
            }
        }
    }, { root: vpEl, threshold: 0.01, rootMargin: '300px 0px' });
    vpi.querySelectorAll('.pwrap').forEach(w => io.observe(w));
    r.io = io;
}
async function reRenderAll(tid) {
    const r = readers.get(tid);
    if (!r)
        return;
    const vpi = $('vpi-' + tid);
    if (!vpi)
        return;
    r.io?.disconnect();
    r.io = null;
    for (const wrap of vpi.querySelectorAll('.pwrap')) {
        const n = parseInt(wrap.dataset['page'] ?? '0', 10);
        const page = await r.pdfDoc.getPage(n);
        const vp = page.getViewport({ scale: r.zoom });
        wrap.style.width = Math.floor(vp.width) + 'px';
        wrap.style.height = Math.floor(vp.height) + 'px';
        wrap.dataset['rendered'] = '';
        wrap.querySelectorAll('canvas').forEach(c => c.remove());
        const tl = wrap.querySelector('.textLayer');
        if (tl)
            tl.innerHTML = '';
        wrap.querySelector('.page-loading')?.remove();
        const ph = document.createElement('div');
        ph.className = 'page-loading';
        ph.innerHTML = `<div class="cspin" style="width:16px;height:16px;border-width:1.5px"></div>`;
        wrap.insertBefore(ph, wrap.firstChild);
    }
    attachObserver(tid);
    restoreHl(tid, r.pdfId);
}
function updateNav(tid) {
    const r = readers.get(tid);
    if (!r)
        return;
    const pind = $('pind-' + tid);
    if (pind)
        pind.textContent = `${r.currentPage} / ${r.pageCount}`;
    const zlbl = $('zlbl-' + tid);
    if (zlbl)
        zlbl.textContent = Math.round(r.zoom * 100) + '%';
    const pg = $('page-' + tid);
    if (!pg)
        return;
    pg.querySelector('[data-a="prev"]').disabled = r.currentPage <= 1;
    pg.querySelector('[data-a="next"]').disabled = r.currentPage >= r.pageCount;
}
function scrollToPage(tid, n) {
    $('vpi-' + tid)?.querySelector(`.pwrap[data-page="${n}"]`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}
function rPrev(tid) { const r = readers.get(tid); if (r && r.currentPage > 1)
    scrollToPage(tid, r.currentPage - 1); }
function rNext(tid) { const r = readers.get(tid); if (r && r.currentPage < r.pageCount)
    scrollToPage(tid, r.currentPage + 1); }
async function rZoom(tid, d) {
    const r = readers.get(tid);
    if (!r)
        return;
    r.zoom = Math.min(3.5, Math.max(0.4, +((r.zoom + d).toFixed(1))));
    updateNav(tid);
    await reRenderAll(tid);
}
function onSel(tid) {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.toString().trim()) {
        hideHlPopup();
        return;
    }
    let pc = null;
    let pn = 0;
    for (let ri = 0; ri < sel.rangeCount; ri++) {
        const rng = sel.getRangeAt(ri);
        const candidate = rng.startContainer.parentElement?.closest('.pwrap');
        if (candidate) {
            pc = candidate;
            pn = parseInt(pc.dataset['page'] ?? '0', 10);
            break;
        }
    }
    if (!pc) {
        hideHlPopup();
        return;
    }
    const pwrapRect = pc.getBoundingClientRect();
    const rects = [];
    for (let ri = 0; ri < sel.rangeCount; ri++) {
        const rng = sel.getRangeAt(ri);
        for (const cr of rng.getClientRects()) {
            if (cr.width < 1 || cr.height < 1)
                continue;
            const x = cr.left - pwrapRect.left;
            const y = cr.top - pwrapRect.top;
            if (x + cr.width < 0 || y + cr.height < 0)
                continue;
            if (x > pwrapRect.width + 10 || y > pwrapRect.height + 10)
                continue;
            rects.push({ x, y, w: cr.width, h: cr.height });
        }
    }
    if (!rects.length) {
        hideHlPopup();
        return;
    }
    selTid = tid;
    const r = readers.get(tid);
    if (r)
        r.selData = { pageNum: pn, rects, tid };
    if (hlModeActive && hlModeTid === tid) {
        applyHighlight();
        return;
    }
    const firstRange = sel.getRangeAt(0);
    const br = firstRange.getBoundingClientRect();
    const popup = $('hl-popup');
    if (popup) {
        popup.style.left = Math.max(8, Math.min(window.innerWidth - 290, br.left + br.width / 2 - 140)) + 'px';
        popup.style.top = Math.max(8, br.top - 52) + 'px';
        popup.classList.add('show');
    }
}
function hideHlPopup() { $('hl-popup')?.classList.remove('show'); selTid = null; }
function toggleHlMode(tid) {
    const pg = $('page-' + tid);
    if (!pg)
        return;
    const btn = pg.querySelector('.hl-mode-btn');
    if (!btn)
        return;
    hlModeActive = !hlModeActive;
    hlModeTid = hlModeActive ? tid : null;
    btn.classList.toggle('active', hlModeActive);
    toast(hlModeActive ? 'Highlight mode ON — select text to highlight instantly' : 'Highlight mode OFF');
}
function buildSettingsSwatches() {
    const row = $('sp-hl-swatches');
    if (!row)
        return;
    row.innerHTML = '';
    HL_COLORS_SP.forEach(c => {
        const sw = document.createElement('div');
        sw.className = 'sp-swatch' + (c === defaultHlColor ? ' sel' : '');
        sw.style.background = c;
        sw.addEventListener('click', () => {
            defaultHlColor = c;
            row.querySelectorAll('.sp-swatch').forEach(s => s.classList.remove('sel'));
            sw.classList.add('sel');
            if (settingsTid) {
                const r = readers.get(settingsTid);
                if (r)
                    r.hlColor = c;
            }
            $('hl-popup')?.querySelectorAll('.hlcb').forEach(b => b.classList.toggle('sel', b.dataset['c'] === c));
        });
        row.appendChild(sw);
    });
}
function toggleSettings(tid) {
    settingsTid = tid;
    const panel = $('settings-panel');
    if (!panel)
        return;
    const showing = panel.classList.contains('show');
    panel.classList.toggle('show', !showing);
    if (!showing) {
        buildSettingsSwatches();
        const r = readers.get(tid);
        const spzv = $('sp-zoom-val');
        if (spzv)
            spzv.textContent = r ? Math.round(r.zoom * 100) + '%' : '—';
        const spfv = $('sp-font-val');
        if (spfv)
            spfv.textContent = globalFontSize + 'px';
    }
}
$('sp-zi')?.addEventListener('click', async () => {
    if (!settingsTid)
        return;
    const r = readers.get(settingsTid);
    if (!r)
        return;
    r.zoom = Math.min(3.5, +((r.zoom + 0.2).toFixed(1)));
    updateNav(settingsTid);
    const spzv = $('sp-zoom-val');
    if (spzv)
        spzv.textContent = Math.round(r.zoom * 100) + '%';
    await reRenderAll(settingsTid);
});
$('sp-zo')?.addEventListener('click', async () => {
    if (!settingsTid)
        return;
    const r = readers.get(settingsTid);
    if (!r)
        return;
    r.zoom = Math.max(0.4, +((r.zoom - 0.2).toFixed(1)));
    updateNav(settingsTid);
    const spzv = $('sp-zoom-val');
    if (spzv)
        spzv.textContent = Math.round(r.zoom * 100) + '%';
    await reRenderAll(settingsTid);
});
$('sp-fd')?.addEventListener('click', () => {
    globalFontSize = Math.min(22, globalFontSize + 1);
    document.body.style.fontSize = globalFontSize + 'px';
    const spfv = $('sp-font-val');
    if (spfv)
        spfv.textContent = globalFontSize + 'px';
});
$('sp-fu')?.addEventListener('click', () => {
    globalFontSize = Math.max(11, globalFontSize - 1);
    document.body.style.fontSize = globalFontSize + 'px';
    const spfv = $('sp-font-val');
    if (spfv)
        spfv.textContent = globalFontSize + 'px';
});
document.addEventListener('click', e => {
    const t = e.target;
    if (!t.closest('#settings-panel') && !t.closest('[data-a="settings"]'))
        $('settings-panel')?.classList.remove('show');
});
$('hl-popup')?.querySelectorAll('.hlcb').forEach(btn => {
    btn.addEventListener('mousedown', e => e.preventDefault());
    btn.addEventListener('click', () => {
        $('hl-popup')?.querySelectorAll('.hlcb').forEach(b => b.classList.remove('sel'));
        btn.classList.add('sel');
        if (selTid) {
            const r = readers.get(selTid);
            if (r)
                r.hlColor = btn.dataset['c'] ?? r.hlColor;
        }
    });
    btn.addEventListener('dblclick', () => {
        $('hl-popup')?.querySelectorAll('.hlcb').forEach(b => b.classList.remove('sel'));
        btn.classList.add('sel');
        if (selTid) {
            const r = readers.get(selTid);
            if (r)
                r.hlColor = btn.dataset['c'] ?? r.hlColor;
        }
        applyHighlight();
    });
});
$('hl-apply')?.addEventListener('mousedown', e => e.preventDefault());
$('hl-apply')?.addEventListener('click', applyHighlight);
$('hl-dismiss')?.addEventListener('click', () => { window.getSelection()?.removeAllRanges(); hideHlPopup(); });
async function applyHighlight() {
    const tid = selTid;
    if (!tid)
        return;
    const r = readers.get(tid);
    if (!r?.selData)
        return;
    const { pageNum, rects } = r.selData;
    const pdfId = r.pdfId;
    if (!appState.highlights[pdfId])
        appState.highlights[pdfId] = [];
    const hlId = gId();
    const color = r.hlColor || '#f9e04b';
    const newHl = { id: hlId, page: pageNum, rects, color };
    appState.highlights[pdfId].push(newHl);
    await save();
    drawHl(tid, pdfId, newHl);
    window.getSelection()?.removeAllRanges();
    hideHlPopup();
}
function drawHl(tid, pdfId, hl) {
    const layer = $('vpi-' + tid)?.querySelector(`.hllayer[data-page="${hl.page}"]`);
    if (!layer)
        return;
    hl.rects.forEach(rect => {
        const d = document.createElement('div');
        d.className = 'hlr';
        d.dataset['hlId'] = hl.id;
        d.style.cssText = `left:${rect.x}px;top:${rect.y}px;width:${rect.w}px;height:${rect.h}px;background:${hl.color};`;
        d.title = 'Click to remove';
        d.addEventListener('click', async (e) => {
            e.stopPropagation();
            appState.highlights[pdfId] = (appState.highlights[pdfId] ?? []).filter(h => h.id !== hl.id);
            $('vpi-' + tid)?.querySelectorAll(`[data-hl-id="${hl.id}"]`).forEach(el => el.remove());
            await save();
        });
        layer.appendChild(d);
    });
}
function restoreHl(tid, pdfId) {
    const hls = appState.highlights[pdfId];
    if (!hls)
        return;
    hls.forEach(hl => drawHl(tid, pdfId, hl));
}
function toggleFind(tid) {
    const b = $('rfind-' + tid);
    if (!b)
        return;
    b.classList.toggle('show');
    if (b.classList.contains('show'))
        setTimeout(() => $('rsi-' + tid)?.focus(), 50);
}
function closeFind(tid) { $('rfind-' + tid)?.classList.remove('show'); clearFind(tid); }
function clearFind(tid) {
    $('vpi-' + tid)?.querySelectorAll('.fhl').forEach(el => { el.style.background = ''; el.style.outline = ''; el.classList.remove('fhl'); });
    const r = readers.get(tid);
    if (r) {
        r.sMatches = [];
        r.sIdx = 0;
    }
    const fi = $('finfo-' + tid);
    if (fi)
        fi.textContent = '';
}
function doFind(tid, q) {
    clearFind(tid);
    if (!q || !readers.get(tid))
        return;
    const ql = q.toLowerCase();
    const matches = [];
    $('vpi-' + tid)?.querySelectorAll('.textLayer span').forEach(s => {
        if (s.textContent?.toLowerCase().includes(ql)) {
            s.style.background = 'rgba(249,224,75,.5)';
            s.classList.add('fhl');
            matches.push(s);
        }
    });
    const r = readers.get(tid);
    if (r) {
        r.sMatches = matches;
        r.sIdx = 0;
    }
    const fi = $('finfo-' + tid);
    if (fi)
        fi.textContent = matches.length ? `${matches.length} found` : 'No results';
    if (matches.length)
        navFind(tid, 1);
}
function navFind(tid, dir) {
    const r = readers.get(tid);
    if (!r || !r.sMatches.length)
        return;
    r.sMatches[r.sIdx]?.style && (r.sMatches[r.sIdx].style.outline = '');
    r.sIdx = (r.sIdx + dir + r.sMatches.length) % r.sMatches.length;
    const el = r.sMatches[r.sIdx];
    if (!el)
        return;
    el.style.outline = '2px solid var(--ac)';
    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
    const fi = $('finfo-' + tid);
    if (fi)
        fi.textContent = `${r.sIdx + 1} / ${r.sMatches.length}`;
}
function getFiltered() {
    let p = appState.pdfs;
    if (activeTopic === '__u')
        p = p.filter(x => !x.topicId);
    else if (activeTopic)
        p = p.filter(x => x.topicId === activeTopic);
    if (searchQ) {
        const q = searchQ.toLowerCase();
        p = p.filter(x => x.name.toLowerCase().includes(q));
    }
    return p;
}
function render() { renderSidebar(); renderContent(); }
function renderSidebar() {
    const list = $('topiclist');
    if (!list)
        return;
    list.innerHTML = '';
    const row = (label, color, count, active, onClick, onDel) => {
        const d = document.createElement('div');
        d.className = 'trow' + (active ? ' active' : '');
        d.innerHTML = `<span class="tc" style="background:${color}"></span><span class="tn">${label}</span><span class="tbadge">${count}</span>${onDel ? `<button class="tdel">${svgI('x', 10, 10)}</button>` : ''}`;
        d.onclick = e => { if (e.target.closest('.tdel'))
            return; onClick(); };
        if (onDel)
            d.querySelector('.tdel')?.addEventListener('click', e => { e.stopPropagation(); onDel(); });
        list.appendChild(d);
    };
    row('All PDFs', '#5e5c59', appState.pdfs.length, activeTopic === null, () => { activeTopic = null; render(); });
    const u = appState.pdfs.filter(p => !p.topicId).length;
    if (u > 0)
        row('Unsorted', '#3e3c39', u, activeTopic === '__u', () => { activeTopic = '__u'; render(); });
    appState.topics.forEach(t => {
        const c = appState.pdfs.filter(p => p.topicId === t.id).length;
        row(esc(t.name), t.color, c, activeTopic === t.id, () => { activeTopic = t.id; render(); }, () => delTopic(t.id));
    });
    const tot = appState.pdfs.length;
    const sbf = $('sbfoot');
    if (sbf)
        sbf.innerHTML = `<strong>${tot}</strong> PDF${tot !== 1 ? 's' : ''} · <strong>${appState.topics.length}</strong> topic${appState.topics.length !== 1 ? 's' : ''}`;
}
function renderContent() {
    const pdfs = getFiltered();
    const empty = pdfs.length === 0;
    const emptyEl = $('empty');
    if (emptyEl)
        emptyEl.style.display = empty ? 'flex' : 'none';
    const pgrid = $('pgrid');
    if (pgrid)
        pgrid.style.display = (!empty && viewMode === 'grid') ? 'grid' : 'none';
    const plist = $('plist');
    if (plist)
        plist.style.display = (!empty && viewMode === 'list') ? 'flex' : 'none';
    let title = 'All PDFs';
    if (activeTopic === '__u')
        title = 'Unsorted';
    else if (activeTopic) {
        const t = appState.topics.find(t => t.id === activeTopic);
        if (t)
            title = `<span class="ti" style="background:${t.color}"></span>${esc(t.name)}`;
    }
    const lt = $('libtitle');
    if (lt)
        lt.innerHTML = title;
    if (empty)
        return;
    viewMode === 'grid' ? renderGrid(pdfs) : renderList(pdfs);
}
function renderGrid(pdfs) {
    const g = $('pgrid');
    if (!g)
        return;
    g.innerHTML = '';
    pdfs.forEach(pdf => {
        const topic = appState.topics.find(t => t.id === pdf.topicId);
        const card = document.createElement('div');
        card.className = 'card';
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
        card.addEventListener('click', e => { if (e.target.closest('.card-menu'))
            return; openPdfTab(pdf); });
        card.querySelector('.card-menu')?.addEventListener('click', e => { e.stopPropagation(); showDD(e.currentTarget, pdf); });
        g.appendChild(card);
        if (coverCache.has(pdf.id))
            stampCover(pdf.id, coverCache.get(pdf.id) ?? null);
        else
            loadCover(pdf);
    });
}
function renderList(pdfs) {
    const l = $('plist');
    if (!l)
        return;
    l.innerHTML = '';
    pdfs.forEach(pdf => {
        const topic = appState.topics.find(t => t.id === pdf.topicId);
        const row = document.createElement('div');
        row.className = 'lrow';
        row.innerHTML = `
      <div class="lthumb" id="lt-${pdf.id}"><svg width="14" height="14" opacity=".3"><use href="#i-doc"/></svg></div>
      <span class="lname">${!pdf.exists ? `<span style="color:var(--danger);margin-right:4px">${svgI('warn', 11, 11)}</span>` : ''}${esc(pdf.name)}</span>
      ${topic ? `<span class="lpill" style="background:${topic.color}22;color:${topic.color}">${esc(topic.name)}</span>` : `<span class="lpill" style="color:var(--t3)">—</span>`}
      <span class="lsize">${fmtSz(pdf.size || 0)}</span>
      <span class="ldate">${fmtDt(pdf.added)}</span>
      <button class="lmenu">${svgI('dots', 13, 13)}</button>`;
        row.addEventListener('click', e => { if (e.target.closest('.lmenu'))
            return; openPdfTab(pdf); });
        row.querySelector('.lmenu')?.addEventListener('click', e => { e.stopPropagation(); showDD(e.currentTarget, pdf); });
        l.appendChild(row);
        if (coverCache.has(pdf.id))
            stampCover(pdf.id, coverCache.get(pdf.id) ?? null);
        else
            loadCover(pdf);
    });
}
$('btn-import')?.addEventListener('click', importPdfs);
async function importPdfs() {
    const files = await invoke('open_pdf_dialog');
    if (!files.length)
        return;
    let added = 0;
    for (const f of files) {
        if (appState.pdfs.find(p => p.path === f.path))
            continue;
        appState.pdfs.push({ id: gId(), path: f.path, name: f.name, size: f.size, added: f.added, topicId: (activeTopic && activeTopic !== '__u') ? activeTopic : null, exists: true });
        added++;
    }
    if (added > 0) {
        await save();
        toast(`Added ${added} PDF${added !== 1 ? 's' : ''}`);
        render();
    }
    else
        toast('Already in library');
}
$('tab-add')?.addEventListener('click', async () => {
    const files = await invoke('open_pdf_dialog');
    if (!files.length)
        return;
    const toOpen = [];
    for (const f of files) {
        let pdf = appState.pdfs.find(p => p.path === f.path);
        if (!pdf) {
            pdf = { id: gId(), path: f.path, name: f.name, size: f.size, added: f.added, topicId: null, exists: true };
            appState.pdfs.push(pdf);
        }
        toOpen.push(pdf);
    }
    await save();
    render();
    toOpen.forEach(p => openPdfTab(p));
});
const cont = $('content');
cont?.addEventListener('dragover', e => { e.preventDefault(); cont.style.outline = '2px dashed var(--ac)'; cont.style.background = 'var(--adim)'; });
cont?.addEventListener('dragleave', () => { cont.style.outline = ''; cont.style.background = ''; });
cont?.addEventListener('drop', async (e) => {
    e.preventDefault();
    cont.style.outline = '';
    cont.style.background = '';
    const dt = e.dataTransfer;
    if (!dt)
        return;
    const files = [...dt.files].filter(f => f.name.toLowerCase().endsWith('.pdf'));
    if (!files.length)
        return;
    const toOpen = [];
    let added = 0;
    for (const f of files) {
        const fp = f.path ?? f.name;
        let pdf = appState.pdfs.find(p => p.path === fp);
        if (!pdf) {
            pdf = { id: gId(), path: fp, name: f.name.replace(/\.pdf$/i, ''), size: f.size, added: Date.now(), topicId: (activeTopic && activeTopic !== '__u') ? activeTopic : null, exists: true };
            appState.pdfs.push(pdf);
            added++;
        }
        toOpen.push(pdf);
    }
    if (added > 0) {
        await save();
        render();
    }
    toOpen.forEach(p => openPdfTab(p));
});
async function removePdf(id) {
    appState.pdfs = appState.pdfs.filter(p => p.id !== id);
    delete appState.highlights[id];
    coverCache.delete(id);
    for (const [tid, m] of tabMeta) {
        if (m.pdfId === id)
            closeTab(tid);
    }
    await save();
    render();
    toast('Removed from library');
}
$('btn-nt')?.addEventListener('click', () => { pendingAssignId = null; openTopicModal(); });
function openTopicModal() {
    tColor = COLORS[Math.floor(Math.random() * COLORS.length)] ?? '#d4a843';
    const tn = $('tname');
    if (tn)
        tn.value = '';
    const cp = $('cpicker');
    if (cp) {
        cp.innerHTML = '';
        COLORS.forEach(c => {
            const sw = document.createElement('div');
            sw.className = 'csw' + (c === tColor ? ' sel' : '');
            sw.style.background = c;
            sw.onclick = () => { tColor = c; cp.querySelectorAll('.csw').forEach(s => s.classList.remove('sel')); sw.classList.add('sel'); };
            cp.appendChild(sw);
        });
    }
    $('mtopic')?.classList.add('show');
    setTimeout(() => $('tname')?.focus(), 100);
}
$('btn-ct')?.addEventListener('click', () => { $('mtopic')?.classList.remove('show'); pendingAssignId = null; });
$('btn-st')?.addEventListener('click', async () => {
    const tn = $('tname');
    const name = tn?.value.trim() ?? '';
    if (!name)
        return;
    const nid = gId();
    appState.topics.push({ id: nid, name, color: tColor });
    if (pendingAssignId) {
        const p = appState.pdfs.find(p => p.id === pendingAssignId);
        if (p)
            p.topicId = nid;
        pendingAssignId = null;
    }
    await save();
    $('mtopic')?.classList.remove('show');
    render();
    toast(`Topic "${name}" created`);
});
$('tname')?.addEventListener('keydown', e => {
    if (e.key === 'Enter')
        $('btn-st')?.click();
    if (e.key === 'Escape')
        $('btn-ct')?.click();
});
async function delTopic(id) {
    appState.pdfs = appState.pdfs.map(p => p.topicId === id ? { ...p, topicId: null } : p);
    appState.topics = appState.topics.filter(t => t.id !== id);
    if (activeTopic === id)
        activeTopic = null;
    await save();
    render();
    toast('Topic deleted');
}
function showDD(anchor, pdf) {
    ddPdf = pdf;
    const menu = $('ddm');
    if (!menu)
        return;
    const tItems = appState.topics.map(t => `<div class="ddi" data-a="assign" data-tid="${t.id}"><span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:${t.color};flex-shrink:0"></span>${esc(t.name)}</div>`).join('');
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
    menu.style.top = (rect.bottom + 4) + 'px';
    menu.style.left = rect.left + 'px';
    menu.classList.add('show');
    requestAnimationFrame(() => {
        const mr = menu.getBoundingClientRect();
        if (mr.right > window.innerWidth - 8)
            menu.style.left = (window.innerWidth - mr.width - 8) + 'px';
        if (mr.bottom > window.innerHeight - 8)
            menu.style.top = (rect.top - mr.height - 4) + 'px';
    });
}
document.addEventListener('click', e => {
    const t = e.target;
    if (!t.closest('#ddm') && !t.closest('.card-menu') && !t.closest('.lmenu'))
        $('ddm')?.classList.remove('show');
    if (!t.closest('#hl-popup') && !t.closest('.textLayer') && !t.closest('.pwrap'))
        hideHlPopup();
});
$('ddm')?.addEventListener('click', async (e) => {
    const item = e.target.closest('[data-a]');
    if (!item || !ddPdf)
        return;
    $('ddm')?.classList.remove('show');
    const a = item.dataset['a'];
    if (a === 'open')
        openPdfTab(ddPdf);
    else if (a === 'assign') {
        ddPdf.topicId = item.dataset['tid'] || null;
        await save();
        render();
        toast(item.dataset['tid'] ? 'Topic assigned' : 'Moved to Unsorted');
    }
    else if (a === 'nt') {
        pendingAssignId = ddPdf.id;
        openTopicModal();
    }
    else if (a === 'rename') {
        const n = prompt('Rename:', ddPdf.name);
        if (n?.trim()) {
            ddPdf.name = n.trim();
            await save();
            render();
        }
    }
    else if (a === 'remove')
        removePdf(ddPdf.id);
});
$('vg')?.addEventListener('click', () => { viewMode = 'grid'; $('vg')?.classList.add('active'); $('vl')?.classList.remove('active'); renderContent(); });
$('vl')?.addEventListener('click', () => { viewMode = 'list'; $('vl')?.classList.add('active'); $('vg')?.classList.remove('active'); renderContent(); });
$('sinput')?.addEventListener('input', e => { searchQ = e.target.value; renderContent(); });
async function checkFiles() {
    let changed = false;
    for (const pdf of appState.pdfs) {
        const e = await invoke('check_exists', { path: pdf.path });
        if (pdf.exists !== e) {
            pdf.exists = e;
            changed = true;
        }
    }
    if (changed) {
        await save();
        render();
    }
}
document.addEventListener('mouseup', () => {
    setTimeout(() => {
        const sel = window.getSelection();
        if (!sel || sel.isCollapsed || !sel.toString().trim())
            return;
        for (const [tid, meta] of tabMeta) {
            if (meta.type !== 'reader')
                continue;
            const vpi = $('vpi-' + tid);
            if (!vpi)
                continue;
            if (sel.anchorNode && vpi.contains(sel.anchorNode)) {
                onSel(tid);
                return;
            }
        }
    }, 10);
});
if (window.__TAURI__) {
    boot();
}
else {
    window.addEventListener('tauri-ready', () => boot(), { once: true });
}
//# sourceMappingURL=renderer.js.map